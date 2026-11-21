//! Window state: start screen → scanning → treemap + side panel.
use crate::theme::{SANS, Tokens, tokens};
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::{IconName, Sizable as _, Theme};
use scan::treemap::Tile;
use scan::{NodeId, Options, Progress, Tree, human};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

pub(crate) const CONTEXT: &str = "DiskApp";
const TOAST_FOR: Duration = Duration::from_secs(4);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);
pub(crate) const SEARCH_LIMIT: usize = 500;
/// Shortcut label shown in the status hint for [`CopyPath`].
pub(crate) const COPY_KEYS: &str = if cfg!(target_os = "macos") { "⌘C" } else { "Ctrl+C" }; // i18n-ok
const SEARCH_KEYS: &str = if cfg!(target_os = "macos") { "⌘F" } else { "Ctrl+F" }; // i18n-ok

actions!(diskmap, [OpenFolder, Rescan, ZoomOut, ZoomIn, ClearSelection, Reveal, MoveToTrash, CopyPath, FocusSearch]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenFolder, Some(CONTEXT)),
        KeyBinding::new("secondary-r", Rescan, Some(CONTEXT)),
        KeyBinding::new("backspace", ZoomOut, Some(CONTEXT)),
        KeyBinding::new("enter", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-r", Reveal, Some(CONTEXT)),
        KeyBinding::new("secondary-backspace", MoveToTrash, Some(CONTEXT)),
        KeyBinding::new("secondary-c", CopyPath, Some(CONTEXT)),
        KeyBinding::new("secondary-f", FocusSearch, Some(CONTEXT)),
    ]);
}

pub(crate) fn sync_theme(window: Option<&mut Window>, cx: &mut App) {
    Theme::sync_system_appearance(window, cx);
    Theme::global_mut(cx).font_family = SANS.into();
}

pub enum Phase {
    Start,
    Scanning { root: PathBuf, progress: Arc<Progress>, started: Instant },
    Ready,
    Failed { root: PathBuf, message: String },
}

pub struct DiskApp {
    pub phase: Phase,
    pub tree: Option<Tree>,
    /// Bumped whenever the tree changes (new scan, removal).
    pub rev: u64,
    scan_gen: u64,
    /// Unreadable entries in the last scan.
    pub scan_errors: u64,
    pub zoom: NodeId,
    pub selected: Option<NodeId>,
    pub hovered: Option<NodeId>,
    pub tiles: Rc<Vec<Tile>>,
    pub tiles_key: Option<(u64, NodeId, i32, i32)>,
    pub canvas_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub largest: Rc<Vec<NodeId>>,
    largest_key: Option<(u64, NodeId)>,
    pub list_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    toast: Option<(String, Instant)>,
    search: Entity<InputState>,
    search_gen: u64,
    placeholder_locale: &'static str,
    /// Applied (debounced, trimmed) query; empty when not searching.
    pub query: String,
    /// Folder in view when the search started; results stay scoped to it while browsing them.
    pub search_root: Option<NodeId>,
    hits: Rc<Vec<NodeId>>,
    hits_key: Option<(u64, NodeId, String)>,
    pub(crate) logo: Arc<Image>,
    pub(crate) about_open: bool,
    /// Other Otherwise apps installed here, found when About opens.
    pub(crate) other_apps: Vec<(otherwise_link::apps::AppSpec, otherwise_link::apps::Installed)>,
    github_stars: bool,
    /// Developer projects, shown instead of the treemap while open.
    pub(crate) dev: Option<crate::dev::DevView>,
    _subs: Vec<Subscription>,
}

impl DiskApp {
    pub fn new(initial: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        sync_theme(Some(window), cx);
        let search = cx.new(|cx| InputState::new(window, cx));
        let subs = vec![
            window.observe_window_appearance(|window, cx| sync_theme(Some(window), cx)),
            cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.query_changed(cx);
                }
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = DiskApp {
            phase: Phase::Start,
            tree: None,
            rev: 0,
            scan_gen: 0,
            scan_errors: 0,
            zoom: Tree::ROOT,
            selected: None,
            hovered: None,
            tiles: Rc::new(Vec::new()),
            tiles_key: None,
            canvas_bounds: Rc::new(Cell::new(Bounds::default())),
            largest: Rc::new(Vec::new()),
            largest_key: None,
            list_scroll: UniformListScrollHandle::new(),
            focus,
            toast: None,
            search,
            search_gen: 0,
            placeholder_locale: "",
            query: String::new(),
            search_root: None,
            hits: Rc::new(Vec::new()),
            hits_key: None,
            about_open: false,
            other_apps: Vec::new(),
            github_stars: crate::settings::Settings::load().github_stars,
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, crate::ICON_PNG.to_vec())),
            dev: None,
            _subs: subs,
        };
        if let Some(p) = initial {
            this.start_scan(p, window, cx);
        }
        crate::star::refresh(this.github_stars, cx);
        this
    }

    pub fn start_scan(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_scan();
        self.close_dev(cx);
        let progress = Arc::new(Progress::default());
        self.scan_gen += 1;
        let scan_gen = self.scan_gen;
        window.set_window_title(&format!("{} — {}", display_name(&root), crate::APP_NAME));
        self.phase = Phase::Scanning { root: root.clone(), progress: progress.clone(), started: Instant::now() };
        let p = progress.clone();
        let task = cx.background_executor().spawn(async move { scan::scan(&root, &Options::default(), &p) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| this.finish_scan(scan_gen, res, &progress, cx)).ok();
        })
        .detach();
        // repaint the live counters while the scan runs
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(120)).await;
                let running = this.update(cx, |this, cx| {
                    let running = this.scan_gen == scan_gen && matches!(this.phase, Phase::Scanning { .. });
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn finish_scan(&mut self, scan_gen: u64, res: std::io::Result<Tree>, progress: &Progress, cx: &mut Context<Self>) {
        if scan_gen != self.scan_gen {
            return;
        }
        match res {
            Ok(tree) => {
                self.tree = Some(tree);
                self.scan_errors = progress.errors.load(Relaxed);
                self.rev += 1;
                self.zoom = Tree::ROOT;
                self.selected = None;
                self.hovered = None;
                self.search_root = self.search_root.map(|_| Tree::ROOT);
                self.phase = Phase::Ready;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                let root = match &self.phase {
                    Phase::Scanning { root, .. } => root.clone(),
                    _ => PathBuf::new(),
                };
                self.phase = Phase::Failed { root, message: io_error(&e) };
            }
        }
        cx.notify();
    }

    fn cancel_scan(&mut self) {
        if let Phase::Scanning { progress, .. } = &self.phase {
            progress.cancel.store(true, Relaxed);
            self.scan_gen += 1;
        }
    }

    fn stop_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_scan();
        self.phase = if self.tree.is_some() { Phase::Ready } else { Phase::Start };
        self.update_title(window);
        cx.notify();
    }

    fn update_title(&self, window: &mut Window) {
        match &self.tree {
            Some(t) if matches!(self.phase, Phase::Ready) => window.set_window_title(&format!("{} — {}", display_name(t.root_path()), crate::APP_NAME)),
            _ => window.set_window_title(crate::APP_NAME),
        }
    }

    pub fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(i18n::t!("app.start.choose_prompt").into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.into_iter().next()
            {
                this.update_in(cx, |this, window, cx| this.start_scan(p, window, cx)).ok();
            }
        })
        .detach();
    }

    fn rescan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = match (&self.phase, &self.tree) {
            (Phase::Failed { root, .. }, _) => root.clone(),
            (_, Some(t)) => t.root_path().to_path_buf(),
            _ => return,
        };
        self.start_scan(root, window, cx);
    }

    pub fn toast(&mut self, text: String, cx: &mut Context<Self>) {
        let at = Instant::now();
        self.toast = Some((text, at));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|(_, t)| *t == at) {
                    this.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // ── navigation ────────────────────────────────────────────────────────────────────────────

    pub fn select(&mut self, id: Option<NodeId>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    /// Double click / Enter: enter a directory, or show a file inside its folder.
    pub fn zoom_into(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        let node = tree.node(id);
        if node.is_dir() && !node.children.is_empty() {
            self.zoom = id;
            self.selected = None;
        } else if let Some(p) = node.parent {
            self.zoom = p;
            self.selected = Some(id);
        }
        self.hovered = None;
        self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn zoom_to(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if self.zoom != id {
            let from = self.zoom;
            self.zoom = id;
            // going up keeps where we came from selected, so it is easy to spot
            self.selected = self.tree.as_ref().filter(|t| t.is_ancestor(id, from) && from != id).map(|t| *t.ancestors(from).get(t.ancestors(id).len()).unwrap_or(&from));
            self.hovered = None;
            cx.notify();
        }
    }

    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.tree.as_ref().and_then(|t| t.node(self.zoom).parent) {
            self.zoom_to(p, cx);
        }
    }

    /// What the panel and actions apply to: the selection, else the zoomed folder.
    pub fn target(&self) -> NodeId {
        self.selected.unwrap_or(self.zoom)
    }

    pub fn reveal(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if let Some(t) = &self.tree {
            cx.reveal_path(&t.path(id));
        }
    }

    pub fn copy_path(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(t) = &self.tree else { return };
        let path = t.path(id).display().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        self.toast(i18n::t!("app.copied", path = path), cx);
    }

    /// Right click on a tile or row: select it and copy its path.
    pub fn select_and_copy(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.select(Some(id), cx);
        self.copy_path(id, cx);
    }

    // ── search ────────────────────────────────────────────────────────────────────────────────

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.search_gen += 1;
        let search_gen = self.search_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.search_gen == search_gen {
                    this.apply_query(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_query(&mut self, cx: &mut Context<Self>) {
        let q = self.search.read(cx).value().trim().to_string();
        if q.is_empty() {
            self.search_root = None;
        } else if self.search_root.is_none() {
            self.search_root = Some(self.zoom);
        }
        self.query = q;
        self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_gen += 1;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.query.clear();
        self.search_root = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn focus_main(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Matches under the search root, largest first; empty when not searching.
    pub fn search_hits(&mut self) -> Rc<Vec<NodeId>> {
        let (Some(tree), Some(root)) = (&self.tree, self.search_root) else { return Rc::new(Vec::new()) };
        if self.query.is_empty() {
            return Rc::new(Vec::new());
        }
        if self.hits_key.as_ref().is_none_or(|(rev, r, q)| *rev != self.rev || *r != root || *q != self.query) {
            self.hits = Rc::new(tree.search(root, &self.query, SEARCH_LIMIT));
            self.hits_key = Some((self.rev, root, self.query.clone()));
        }
        self.hits.clone()
    }

    /// Show a result in the treemap: zoom to its folder and select it.
    pub fn select_hit(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(parent) = self.tree.as_ref().and_then(|t| t.node(id).parent) else { return };
        self.zoom_to(parent, cx);
        self.selected = Some(id);
        cx.notify();
    }

    pub fn largest_files(&mut self) -> Rc<Vec<NodeId>> {
        let key = (self.rev, self.zoom);
        if self.largest_key != Some(key)
            && let Some(t) = &self.tree
        {
            self.largest = Rc::new(t.largest_files(self.zoom, 8));
            self.largest_key = Some(key);
        }
        self.largest.clone()
    }

    pub fn move_to_trash(&mut self, id: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tree) = &self.tree else { return };
        if id == Tree::ROOT {
            return;
        }
        let path = tree.path(id);
        let node = tree.node(id);
        let size = human(node.size);
        let msg = i18n::t!("app.trash.confirm", name = node.name);
        let detail = if node.is_dir() { i18n::tn!("app.trash.detail_dir", node.files, size = size) } else { i18n::t!("app.trash.detail_file", size = size) };
        let (ok, cancel) = (i18n::t!("app.trash.confirm_button"), i18n::t!("common.cancel"));
        let rx = window.prompt(PromptLevel::Warning, &msg, Some(&detail), &[ok.as_str(), cancel.as_str()], cx);
        let scan_gen = self.scan_gen;
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let p = path.clone();
            let res = cx.background_executor().spawn(async move { trash::delete(&p) }).await;
            this.update(cx, |this, cx| match res {
                Ok(()) => this.removed(scan_gen, id, size, cx),
                Err(e) => this.toast(i18n::t!("app.trash.failed", detail = e), cx),
            })
            .ok();
        })
        .detach();
    }

    fn removed(&mut self, scan_gen: u64, id: NodeId, size: String, cx: &mut Context<Self>) {
        if scan_gen != self.scan_gen {
            return;
        }
        let Some(tree) = &mut self.tree else { return };
        let name = tree.node(id).name.to_string();
        let parent = tree.node(id).parent.unwrap_or(Tree::ROOT);
        if tree.is_ancestor(id, self.zoom) {
            self.zoom = parent;
        }
        if self.selected.is_some_and(|s| tree.is_ancestor(id, s)) {
            self.selected = None;
        }
        if self.search_root.is_some_and(|r| tree.is_ancestor(id, r)) {
            self.search_root = Some(parent);
        }
        tree.remove(id);
        self.hovered = None;
        self.rev += 1;
        self.toast(i18n::t!("app.trash.done", name = name, size = size), cx);
    }

    fn set_language(&mut self, code: &'static str, cx: &mut Context<Self>) {
        if i18n::set_locale(code) {
            let mut s = crate::settings::Settings::load();
            s.language = Some(code.to_string());
            s.save();
            gpui_component::set_locale(code);
            crate::set_app_menus(cx);
            cx.refresh_windows();
        }
    }

    fn set_github_stars(&mut self, on: bool, cx: &mut Context<Self>) {
        self.github_stars = on;
        let mut s = crate::settings::Settings::load();
        s.github_stars = on;
        s.save();
        crate::star::refresh(on, cx);
        cx.notify();
    }

    // ── rendering ─────────────────────────────────────────────────────────────────────────────

    fn render_topbar(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let mut crumbs = div().flex().items_center().gap(px(2.)).min_w_0().flex_1().overflow_hidden();
        let mut total = None;
        if let (Some(tree), Phase::Ready) = (&self.tree, &self.phase) {
            let path = tree.ancestors(self.zoom);
            let last = path.len() - 1;
            for (i, id) in path.into_iter().enumerate() {
                if i > 0 {
                    crumbs = crumbs.child(icon(IconName::ChevronRight, 14., t.ink_muted).flex_none());
                }
                let name = if i == 0 { tree.root_path().display().to_string() } else { tree.node(id).name.to_string() };
                let hover = t.canvas;
                crumbs = crumbs.child(
                    div()
                        .id(("crumb", i))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .truncate()
                        .when(i == last, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(t.ink).flex_shrink_0())
                        .when(i != last, |d| d.text_color(t.ink_muted).max_w(px(220.)).cursor_pointer().hover(move |s| s.bg(hover)))
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(id, cx))),
                );
            }
            let n = tree.node(self.zoom);
            total = Some(div().flex_none().text_size(px(13.)).text_color(t.ink_muted).child(i18n::tn!("app.topbar.total", n.files, size = human(n.size))));
        } else {
            crumbs = crumbs.child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(crate::APP_NAME));
        }
        let can_up = matches!(self.phase, Phase::Ready) && self.tree.as_ref().is_some_and(|t| t.node(self.zoom).parent.is_some());
        let can_rescan = matches!(self.phase, Phase::Ready | Phase::Failed { .. });
        let current = i18n::locale();
        let weak = cx.entity().downgrade();
        let stars_on = self.github_stars;
        let lang = Button::new("lang").ghost().icon(IconName::Globe).with_size(px(34.)).tooltip(i18n::t!("common.language")).dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            menu = menu.label(i18n::t!("common.language"));
            for (code, name) in i18n::available() {
                let w = weak.clone();
                menu = menu.item(PopupMenuItem::new(name).checked(code == current).on_click(move |_, _, cx| {
                    w.update(cx, |this, cx| this.set_language(code, cx)).ok();
                }));
            }
            let w = weak.clone();
            menu = menu.separator().item(PopupMenuItem::new(i18n::t!("app.star.setting")).checked(stars_on).on_click(move |_, _, cx| {
                w.update(cx, |this, cx| this.set_github_stars(!stars_on, cx)).ok();
            }));
            menu = menu.label(i18n::t!("app.star.setting_hint"));
            let w = weak.clone();
            menu.separator().item(PopupMenuItem::new(i18n::t!("app.credit.by", company = crate::COMPANY)).on_click(move |_, window, cx| {
                w.update(cx, |this, cx| this.show_about(window, cx)).ok();
            }))
            .item(PopupMenuItem::new(i18n::t!("app.menu.reveal_data")).on_click(|_, _, cx| crate::storage::reveal_data_dir(cx)))
            .item(PopupMenuItem::new(crate::vcity::menu_label()).on_click(|_, _, cx| crate::vcity::open(cx)))
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(52.))
            .px(px(12.))
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(div().flex().flex_none().items_center().justify_center().size(px(30.)).rounded(px(8.)).bg(t.brand).child(svg().path(crate::LOGO_SVG).size(px(16.)).text_color(gpui::white())))
            .child(ui::icon_button(t, "up", IconName::ArrowUp, can_up).when(can_up, |d| d.on_click(cx.listener(|this, _, _, cx| this.zoom_out(cx)))))
            .child(crumbs)
            .children(total)
            .when(matches!(self.phase, Phase::Ready), |d| d.child(Input::new(&self.search).prefix(icon(IconName::Search, 14., t.ink_muted)).cleanable(true).w(px(220.)).flex_none()))
            .child(ui::icon_button(t, "rescan", IconName::Redo, can_rescan).when(can_rescan, |d| d.on_click(cx.listener(|this, _, window, cx| this.rescan(window, cx)))))
            .child(ui::outline_button(t, "dev").child(icon(IconName::SquareTerminal, 15., t.ink)).child(i18n::t!("app.dev.menu")).on_click(cx.listener(|this, _, window, cx| this.open_dev(window, cx))))
            .child(ui::outline_button(t, "open").child(icon(IconName::FolderOpen, 15., t.ink)).child(i18n::t!("app.topbar.open")).on_click(cx.listener(|this, _, window, cx| this.open_dialog(window, cx))))
            .child(lang)
    }

    fn render_start(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let downloads = home.as_ref().map(|h| h.join("Downloads")).filter(|p| p.is_dir());
        let disk = if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") };
        let place = |id: &'static str, ic: IconName, label: String, path: Option<PathBuf>| {
            path.map(|p| {
                let hint = p.display().to_string();
                ui::outline_button(t, id)
                    .h(px(64.))
                    .w(px(220.))
                    .gap(px(12.))
                    .child(icon(ic, 20., t.accent))
                    .child(div().flex().flex_col().min_w_0().child(div().child(label)).child(div().text_size(px(11.)).text_color(t.ink_muted).font_weight(FontWeight::NORMAL).truncate().child(hint)))
                    .on_click(cx.listener(move |this, _, window, cx| this.start_scan(p.clone(), window, cx)))
            })
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(24.))
            .size_full()
            .child(img(self.logo.clone()).size(px(112.)))
            .child(div().flex().flex_col().items_center().gap(px(6.)).child(div().text_size(px(26.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(i18n::t!("app.start.title"))).child(div().text_size(px(14.)).text_color(t.ink_muted).child(i18n::t!("app.start.subtitle"))))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(12.))
                    .max_w(px(720.))
                    .children(place("home", IconName::CircleUser, i18n::t!("app.start.home"), home))
                    .children(place("downloads", IconName::Inbox, i18n::t!("app.start.downloads"), downloads))
                    .children(place("disk", IconName::HardDrive, i18n::t!("app.start.disk"), Some(disk)))
                    .child(
                        ui::outline_button(t, "dev-start")
                            .h(px(64.))
                            .w(px(220.))
                            .gap(px(12.))
                            .child(icon(IconName::SquareTerminal, 20., t.accent))
                            .child(div().flex().flex_col().min_w_0().child(div().child(i18n::t!("app.dev.title"))).child(div().text_size(px(11.)).text_color(t.ink_muted).font_weight(FontWeight::NORMAL).truncate().child(i18n::t!("app.dev.start_hint"))))
                            .on_click(cx.listener(|this, _, window, cx| this.open_dev(window, cx))),
                    ),
            )
            .child(ui::filled_button(t, "choose", t.accent).child(icon(IconName::FolderOpen, 15., t.on_fill())).child(i18n::t!("app.start.choose")).on_click(cx.listener(|this, _, window, cx| this.open_dialog(window, cx))))
            .when(cfg!(target_os = "macos"), |d| d.child(div().max_w(px(460.)).text_center().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.start.mac_hint"))))
            .children(self.render_explorer_toggle(t, cx))
    }

    #[cfg(windows)]
    fn render_explorer_toggle(&self, t: &Tokens, cx: &mut Context<Self>) -> Option<AnyElement> {
        let installed = crate::explorer::is_installed();
        let label = if installed { i18n::t!("app.explorer.remove") } else { i18n::t!("app.explorer.add") };
        let button = ui::outline_button(t, "explorer-menu").child(icon(IconName::FolderOpen, 14., t.ink_muted)).child(label).on_click(cx.listener(move |this, _, _, cx| {
            let res = if installed { crate::explorer::uninstall() } else { crate::explorer::install() };
            match res {
                Ok(()) => this.toast(if installed { i18n::t!("app.explorer.removed") } else { i18n::t!("app.explorer.added") }, cx),
                Err(e) => this.toast(i18n::t!("app.explorer.failed", detail = e), cx),
            }
            cx.notify();
        }));
        Some(button.into_any_element())
    }

    #[cfg(not(windows))]
    fn render_explorer_toggle(&self, _: &Tokens, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn render_scanning(&self, t: &Tokens, root: &Path, p: &Progress, started: Instant, cx: &mut Context<Self>) -> impl IntoElement {
        let secs = started.elapsed().as_secs_f32();
        let stat = |label: String, value: String| div().flex().flex_col().items_center().gap(px(2.)).w(px(130.)).child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(value)).child(div().text_size(px(12.)).text_color(t.ink_muted).child(label));
        let files = p.files.load(Relaxed);
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(20.))
            .size_full()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(i18n::t!("app.scan.title")))
            .child(div().max_w(px(560.)).truncate().text_size(px(13.)).text_color(t.ink_muted).child(root.display().to_string()))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .p(px(16.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.surface)
                    .child(stat(i18n::t!("app.scan.files"), group(files)))
                    .child(stat(i18n::t!("app.scan.folders"), group(p.dirs.load(Relaxed))))
                    .child(stat(i18n::t!("app.scan.size"), human(p.bytes.load(Relaxed))))
                    .child(stat(i18n::t!("app.scan.elapsed"), format!("{secs:.1} s"))),
            )
            .child(div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.scan.rate", rate = group((files as f32 / secs.max(0.1)) as u64))))
            .child(ui::outline_button(t, "cancel").child(i18n::t!("common.cancel")).on_click(cx.listener(|this, _, window, cx| this.stop_scan(window, cx))))
    }

    fn render_failed(&self, t: &Tokens, root: &Path, message: &str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .size_full()
            .child(icon(IconName::TriangleAlert, 32., t.danger))
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(i18n::t!("app.error.title")))
            .child(div().text_size(px(13.)).text_color(t.ink_muted).child(root.display().to_string()))
            .child(div().max_w(px(520.)).text_center().text_size(px(13.)).text_color(t.ink).child(message.to_string()))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(ui::outline_button(t, "back").child(i18n::t!("app.error.back")).on_click(cx.listener(|this, _, window, cx| {
                        this.phase = if this.tree.is_some() { Phase::Ready } else { Phase::Start };
                        this.update_title(window);
                        cx.notify();
                    })))
                    .child(ui::filled_button(t, "retry", t.accent).child(i18n::t!("app.error.retry")).on_click(cx.listener(|this, _, window, cx| this.rescan(window, cx)))),
            )
    }

    fn render_toast(&self, t: &Tokens) -> Option<impl IntoElement> {
        let (text, _) = self.toast.as_ref()?;
        Some(div().absolute().bottom(px(40.)).left_0().right_0().flex().justify_center().child(div().px(px(16.)).py(px(10.)).rounded(px(12.)).bg(t.ink).text_color(t.on_fill()).text_size(px(13.)).child(text.clone())))
    }
}

impl Render for DiskApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx);
        if self.placeholder_locale != i18n::locale() {
            self.placeholder_locale = i18n::locale();
            let text = i18n::t!("app.search.placeholder", keys = SEARCH_KEYS);
            self.search.update(cx, |s, cx| s.set_placeholder(text, window, cx));
        }
        let body: AnyElement = if self.dev.is_some() { self.render_dev(&t, cx) } else { match &self.phase {
            Phase::Start => self.render_start(&t, cx).into_any_element(),
            Phase::Scanning { root, progress, started } => {
                let (root, progress, started) = (root.clone(), progress.clone(), *started);
                self.render_scanning(&t, &root, &progress, started, cx).into_any_element()
            }
            Phase::Failed { root, message } => {
                let (root, message) = (root.clone(), message.clone());
                self.render_failed(&t, &root, &message, cx).into_any_element()
            }
            Phase::Ready => div().flex().size_full().min_h_0().child(self.render_treemap(window, cx)).child(self.render_panel(&t, cx)).into_any_element(),
        } };
        let ready = matches!(self.phase, Phase::Ready) || self.dev.is_some();
        let about = self.about_open;
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| {
                if this.about_open {
                    this.close_about(cx);
                } else if !this.query.is_empty() || this.search_focused(window, cx) {
                    this.clear_search(window, cx);
                } else {
                    this.select(None, cx);
                }
            }))
            // the modal swallows every other shortcut, e.g. ⌘⌫ must not trash what sits behind it
            .when(!about && self.dev.is_none(), |d| {
                d.on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_dialog(window, cx)))
                .on_action(cx.listener(|this, _: &Rescan, window, cx| this.rescan(window, cx)))
                .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_out(cx)))
                .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                    if this.search_focused(window, cx) {
                        this.apply_query(cx);
                        if let Some(&first) = this.search_hits().first() {
                            this.select_hit(first, cx);
                        }
                    } else if let Some(s) = this.selected {
                        this.zoom_into(s, cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                    if matches!(this.phase, Phase::Ready) {
                        this.search.update(cx, |s, cx| s.focus(window, cx));
                    }
                }))
                .on_action(cx.listener(|this, _: &Reveal, _, cx| {
                    let id = this.target();
                    this.reveal(id, cx);
                }))
                .on_action(cx.listener(|this, _: &CopyPath, _, cx| {
                    if matches!(this.phase, Phase::Ready) {
                        let id = this.target();
                        this.copy_path(id, cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &MoveToTrash, window, cx| {
                    if let Some(s) = this.selected {
                        this.move_to_trash(s, window, cx);
                    }
                }))
            })
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.canvas)
            .text_color(t.ink)
            .font_family(SANS)
            .child(self.render_topbar(&t, cx))
            .child(div().relative().flex_1().min_h_0().child(body).when(!ready, |d| d.child(div().absolute().bottom(px(18.)).left_0().right_0().flex().justify_center().child(self.render_footer(&t, "footer", cx)))))
            .children(self.render_toast(&t))
            .when(about, |d| d.child(self.render_about(&t, cx)))
    }
}

pub fn display_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// 1234567 → "1 234 567" (thin no-break space, readable in every bundled language).
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(ch);
    }
    out
}

/// OS error text is in the system language, so the common cases get our own wording.
fn io_error(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => i18n::t!("app.error.not_found"),
        PermissionDenied => i18n::t!("app.error.permission"),
        NotADirectory => i18n::t!("app.error.not_dir"),
        _ => i18n::t!("app.error.other", detail = e),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn groups_digits() {
        assert_eq!(super::group(0), "0");
        assert_eq!(super::group(999), "999");
        assert_eq!(super::group(1234567), "1\u{202F}234\u{202F}567");
    }
}
