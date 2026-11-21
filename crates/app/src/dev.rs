//! Developer projects: build folders (`target/`, `node_modules/`, `.next/`, Gradle, DerivedData)
//! and package caches found under some roots (`scan::dev::find`), grouped by project with their
//! last change, "Open in rama" for git repositories and the worktrees rama knows marked. Cleanup
//! deletes only regenerable folders, after an in-app list of every path and size and a second,
//! native confirmation; `scan::dev::delete` re-checks each one.
use crate::app::DiskApp;
use crate::theme::Tokens;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::IconName;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputState};
use scan::dev::{self, Artifact, ArtifactKind, Project, Refusal, Report};
use scan::{Progress, human};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

/// Projects (and build caches) unchanged this long are pre-selected.
pub const STALE_DAYS: i64 = 14;

pub enum DevState {
    Scanning { progress: Arc<Progress>, started: Instant },
    Ready(Rc<Report>),
}

pub struct DevView {
    pub roots: Entity<InputState>,
    pub state: DevState,
    pub selected: HashSet<PathBuf>,
    pub expanded: HashSet<PathBuf>,
    /// Worktree folders rama has listed.
    pub known: HashSet<PathBuf>,
    pub rama: bool,
    pub confirm: bool,
    pub running: bool,
    /// (freed, left alone with why) of the last cleanup.
    pub result: Option<(u64, Vec<(PathBuf, String)>)>,
    generation: u64,
}

fn home() -> PathBuf {
    otherwise_link::paths::home()
}

fn roots_file() -> PathBuf {
    crate::storage::app_dir().join("dev-roots.txt")
}

/// The folders searched: saved ones, else the home folder.
pub fn load_roots() -> Vec<PathBuf> {
    let saved: Vec<PathBuf> = std::fs::read_to_string(roots_file()).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    if saved.is_empty() { vec![home()] } else { saved }
}

/// `~/code, /Volumes/Work` → absolute folders (`~` expanded, blanks dropped).
pub fn parse_roots(text: &str, home: &Path) -> Vec<PathBuf> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_prefix("~") {
            Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(s),
        })
        .filter(|p| p.is_absolute())
        .collect()
}

fn days_since(t: i64, now: i64) -> i64 {
    if t <= 0 { i64::MAX } else { ((now - t).max(0)) / 86_400 }
}

/// Pre-selected: build folders of projects untouched for [`STALE_DAYS`] and build caches not
/// used as long; never package download caches (they cost a download, not a build).
pub fn default_selection(r: &Report, now: i64) -> HashSet<PathBuf> {
    let mut s = HashSet::new();
    for p in r.projects.iter().filter(|p| days_since(p.modified, now) >= STALE_DAYS) {
        s.extend(p.artifacts.iter().map(|a| a.path.clone()));
    }
    s.extend(r.caches.iter().filter(|c| !c.kind.is_download_cache() && days_since(c.modified, now) >= STALE_DAYS).map(|c| c.path.clone()));
    s
}

/// Every selected folder with the project it belongs to (`None`: a home cache), in list order.
pub fn plan<'a>(r: &'a Report, selected: &HashSet<PathBuf>) -> Vec<(&'a Artifact, Option<&'a Project>)> {
    let mut out: Vec<(&Artifact, Option<&Project>)> = Vec::new();
    for p in &r.projects {
        out.extend(p.artifacts.iter().filter(|a| selected.contains(&a.path)).map(|a| (a, Some(p))));
    }
    out.extend(r.caches.iter().filter(|c| selected.contains(&c.path)).map(|c| (c, None)));
    out
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn kind_label(k: ArtifactKind) -> String {
    i18n::t!(&format!("app.dev.kind.{}", k.id()))
}

fn age(t: i64) -> String {
    match days_since(t, now()) {
        i64::MAX => String::new(),
        0 => i18n::t!("app.dev.today"),
        d => i18n::tn!("app.dev.days_ago", d as u64),
    }
}

impl DiskApp {
    pub fn open_dev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dev.is_none() {
            let text = load_roots().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
            let roots = cx.new(|cx| InputState::new(window, cx).default_value(text));
            self.dev = Some(DevView { roots, state: DevState::Ready(Rc::new(Report::default())), selected: HashSet::new(), expanded: HashSet::new(), known: HashSet::new(), rama: false, confirm: false, running: false, result: None, generation: 0 });
            self.scan_dev(cx);
        }
        cx.notify();
    }

    pub fn close_dev(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.dev.take()
            && let DevState::Scanning { progress, .. } = &d.state
        {
            progress.cancel.store(true, Relaxed);
        }
        cx.notify();
    }

    fn scan_dev(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.dev.as_mut() else { return };
        if let DevState::Scanning { progress, .. } = &d.state {
            progress.cancel.store(true, Relaxed);
        }
        let home = home();
        let roots = parse_roots(&d.roots.read(cx).value(), &home);
        let roots = if roots.is_empty() { vec![home.clone()] } else { roots };
        let _ = std::fs::create_dir_all(crate::storage::app_dir());
        let _ = std::fs::write(roots_file(), roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        let progress = Arc::new(Progress::default());
        d.generation += 1;
        let generation = d.generation;
        d.state = DevState::Scanning { progress: progress.clone(), started: Instant::now() };
        d.result = None;
        let p = progress.clone();
        let first = roots[0].clone();
        let task = cx.background_executor().spawn(async move {
            let report = dev::find(&roots, Some(&home), &p);
            let known = crate::otherwise::rama_worktrees(&crate::otherwise::rama_worktrees_file());
            let rama = crate::otherwise::rama_installed();
            (report, known, rama)
        });
        cx.spawn(async move |this, cx| {
            let (report, known, rama) = task.await;
            let cancelled = progress.cancel.load(Relaxed);
            this.update(cx, |this, cx| {
                let Some(d) = this.dev.as_mut().filter(|d| d.generation == generation) else { return };
                if !cancelled {
                    let summary = crate::otherwise::DevSummary { root: first, bytes: report.build_bytes(), at: now() };
                    std::thread::spawn(move || {
                        summary.save();
                        let _ = crate::otherwise::publish_today(&otherwise_link::Root::system());
                    });
                }
                d.selected = default_selection(&report, now());
                d.known = known;
                d.rama = rama;
                d.state = DevState::Ready(Rc::new(report));
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let running = this.update(cx, |this, cx| {
                    let running = this.dev.as_ref().is_some_and(|d| d.generation == generation && matches!(d.state, DevState::Scanning { .. }));
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

    fn dev_toggle(&mut self, paths: Vec<PathBuf>, on: bool, cx: &mut Context<Self>) {
        if let Some(d) = self.dev.as_mut() {
            for p in paths {
                if on {
                    d.selected.insert(p);
                } else {
                    d.selected.remove(&p);
                }
            }
        }
        cx.notify();
    }

    /// The second confirmation (native), then the deletes in the background.
    fn dev_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.dev.as_mut() else { return };
        let DevState::Ready(report) = &d.state else { return };
        let report = report.clone();
        let items: Vec<(Artifact, Option<Project>)> = plan(&report, &d.selected).into_iter().map(|(a, p)| (a.clone(), p.cloned())).collect();
        if items.is_empty() || d.running {
            return;
        }
        let bytes: u64 = items.iter().map(|(a, _)| a.bytes).sum();
        let msg = i18n::tn!("app.dev.confirm2", items.len() as u64, size = human(bytes));
        let detail = i18n::t!("app.dev.confirm2_detail");
        let (ok, cancel) = (i18n::t!("app.dev.delete_ok"), i18n::t!("common.cancel"));
        let rx = window.prompt(PromptLevel::Critical, &msg, Some(&detail), &[ok.as_str(), cancel.as_str()], cx);
        cx.spawn(async move |this, cx| {
            if rx.await != Ok(0) {
                return;
            }
            let go = this.update(cx, |this, cx| {
                if let Some(d) = this.dev.as_mut() {
                    d.running = true;
                    d.confirm = false;
                }
                cx.notify();
            });
            if go.is_err() {
                return;
            }
            let home = home();
            let res = cx
                .background_executor()
                .spawn(async move {
                    let mut freed = 0;
                    let mut skipped = Vec::new();
                    for (a, p) in &items {
                        match dev::delete(a, p.as_ref(), Some(&home)) {
                            Ok(()) => freed += a.bytes,
                            Err(Refusal::NotArtifact) => skipped.push((a.path.clone(), i18n::t!("app.dev.skip_not_artifact"))),
                            Err(Refusal::Changed) => skipped.push((a.path.clone(), i18n::t!("app.dev.skip_changed"))),
                            Err(Refusal::Failed(e)) => skipped.push((a.path.clone(), e)),
                        }
                    }
                    (freed, skipped)
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(d) = this.dev.as_mut() {
                    d.running = false;
                    d.result = Some(res);
                }
                this.rescan_dev_after_cleanup(cx);
            })
            .ok();
        })
        .detach();
    }

    fn rescan_dev_after_cleanup(&mut self, cx: &mut Context<Self>) {
        let result = self.dev.as_mut().and_then(|d| d.result.take());
        self.scan_dev(cx);
        if let Some(d) = self.dev.as_mut() {
            d.result = result;
        }
    }

    fn open_rama(&mut self, repo: PathBuf, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move { crate::otherwise::open_in_rama(&repo).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                this.update(cx, |this, cx| this.toast(i18n::t!("app.dev.rama_failed", detail = e), cx)).ok();
            }
        })
        .detach();
    }

    pub(crate) fn render_dev(&mut self, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.dev.as_ref() else { return div().into_any_element() };
        let header = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(8.))
            .px(px(20.))
            .py(px(14.))
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon(IconName::SquareTerminal, 18., t.accent))
                    .child(div().flex_1().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.dev.title")))
                    .child(ui::icon_button(t, "dev-close", IconName::Close, true).on_click(cx.listener(|this, _, _, cx| this.close_dev(cx)))),
            )
            .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(t.ink_muted).child(i18n::t!("app.dev.subtitle")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_none().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.dev.roots")))
                    .child(div().flex_1().child(Input::new(&d.roots)))
                    .child(ui::outline_button(t, "dev-rescan").child(icon(IconName::Redo, 14., t.ink)).child(i18n::t!("app.dev.rescan")).on_click(cx.listener(|this, _, _, cx| this.scan_dev(cx)))),
            );
        let body: AnyElement = match &d.state {
            DevState::Scanning { progress, started } => div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.dev.scanning")))
                .child(div().text_size(px(12.5)).text_color(t.ink_muted).child(i18n::t!("app.dev.scanning_stats", dirs = crate::app::group(progress.dirs.load(Relaxed)), size = human(progress.bytes.load(Relaxed)), secs = format!("{:.0}", started.elapsed().as_secs_f32()))))
                .child(ui::outline_button(t, "dev-stop").child(i18n::t!("common.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(DevState::Scanning { progress, .. }) = this.dev.as_ref().map(|d| &d.state) {
                        progress.cancel.store(true, Relaxed);
                    }
                    cx.notify();
                })))
                .into_any_element(),
            DevState::Ready(report) => self.render_dev_list(t, report.clone(), cx),
        };
        let dialog = d.confirm.then(|| self.render_dev_confirm(t, cx));
        div().relative().flex().flex_col().size_full().child(header).child(div().flex().flex_col().flex_1().min_h_0().child(body)).children(dialog).into_any_element()
    }

    fn render_dev_list(&self, t: &Tokens, report: Rc<Report>, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.dev.as_ref() else { return div().into_any_element() };
        let caches: u64 = report.caches.iter().map(|c| c.bytes).sum();
        let builds: u64 = report.projects.iter().map(|p| p.bytes).sum();
        let mut list = div().id("dev-list").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().px(px(20.)).py(px(12.)).gap(px(4.));
        if let Some((freed, skipped)) = &d.result {
            let mut note = div().flex().flex_col().gap(px(4.)).p(px(12.)).mb(px(8.)).rounded(px(10.)).bg(t.accent_soft).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.dev.freed", size = human(*freed))));
            for (p, why) in skipped {
                note = note.child(div().text_size(px(11.5)).text_color(t.ink_muted).child(i18n::t!("app.dev.skipped", path = p.display(), why = why)));
            }
            list = list.child(note);
        }
        list = list.child(div().text_size(px(13.)).text_color(t.ink_muted).pb(px(6.)).child(i18n::tn!("app.dev.summary", report.projects.len() as u64, builds = human(builds), caches = human(caches))));
        if report.projects.is_empty() && report.caches.is_empty() {
            list = list.child(div().py(px(30.)).text_center().text_size(px(13.)).text_color(t.ink_muted).child(i18n::t!("app.dev.none")));
        }
        let row_artifact = |a: &Artifact, rel: String, on: bool, i: usize, cx: &mut Context<Self>| {
            let path = a.path.clone();
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(28.))
                .py(px(2.))
                .child(Checkbox::new(("dev-art", i)).checked(on).on_click(cx.listener(move |this, on: &bool, _, cx| this.dev_toggle(vec![path.clone()], *on, cx))))
                .child(div().flex_none().w(px(150.)).text_size(px(12.)).child(kind_label(a.kind)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(11.5)).text_color(t.ink_muted).child(rel))
                .child(div().flex_none().w(px(110.)).text_size(px(11.5)).text_color(t.ink_muted).child(age(a.modified)))
                .child(div().flex_none().w(px(80.)).text_right().text_size(px(12.5)).child(human(a.bytes)))
        };
        let mut n = 0usize;
        for (pi, p) in report.projects.iter().enumerate() {
            let paths: Vec<PathBuf> = p.artifacts.iter().map(|a| a.path.clone()).collect();
            let all = paths.iter().all(|x| d.selected.contains(x));
            let open = d.expanded.contains(&p.root);
            let name = p.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.root.display().to_string());
            let chip = |text: String, c: Hsla| div().flex_none().px(px(6.)).rounded(px(5.)).bg(c.opacity(0.14)).text_color(c).text_size(px(10.5)).child(text);
            let root = p.root.clone();
            let toggle_root = p.root.clone();
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(5.))
                .border_t_1()
                .border_color(t.border)
                .child(Checkbox::new(("dev-proj", pi)).checked(all).on_click(cx.listener(move |this, on: &bool, _, cx| this.dev_toggle(paths.clone(), *on, cx))))
                .child(
                    div()
                        .id(("dev-expand", pi))
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 13., t.ink_muted))
                        .child(div().flex_none().max_w(px(260.)).truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(name))
                        .when(p.worktree, |d| d.child(chip(i18n::t!("app.dev.worktree"), t.accent)))
                        .when(p.git && !p.worktree, |d| d.child(chip(i18n::t!("app.dev.git"), t.ink_muted)))
                        .when(d.known.contains(&p.root), |d| d.child(chip(i18n::t!("app.dev.in_rama"), t.brand)))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(11.5)).text_color(t.ink_muted).child(p.root.display().to_string()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.dev.as_mut()
                                && !d.expanded.remove(&toggle_root)
                            {
                                d.expanded.insert(toggle_root.clone());
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_none().w(px(110.)).text_size(px(11.5)).text_color(t.ink_muted).child(age(p.modified)));
            if d.rama && p.git {
                let r = root.clone();
                row = row.child(ui::outline_button(t, ("dev-rama", pi)).h(px(24.)).px(px(8.)).text_size(px(11.5)).child(i18n::t!("app.dev.open_rama")).on_click(cx.listener(move |this, _, _, cx| this.open_rama(r.clone(), cx))));
            }
            let s = root.clone();
            row = row
                .child(ui::outline_button(t, ("dev-show", pi)).h(px(24.)).px(px(8.)).text_size(px(11.5)).child(i18n::t!("app.dev.show")).on_click(cx.listener(move |this, _, window, cx| {
                    this.close_dev(cx);
                    this.start_scan(s.clone(), window, cx);
                })))
                .child(div().flex_none().w(px(80.)).text_right().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(human(p.bytes)));
            list = list.child(row);
            if open {
                for a in &p.artifacts {
                    let rel = a.path.strip_prefix(&p.root).map(|r| r.display().to_string()).unwrap_or_else(|_| a.path.display().to_string());
                    list = list.child(row_artifact(a, rel, d.selected.contains(&a.path), n, cx));
                    n += 1;
                }
            }
        }
        if !report.caches.is_empty() {
            list = list.child(div().pt(px(16.)).pb(px(4.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.dev.caches"))).child(div().pb(px(4.)).text_size(px(11.5)).text_color(t.ink_muted).child(i18n::t!("app.dev.caches_hint")));
            for c in &report.caches {
                list = list.child(row_artifact(c, c.path.display().to_string(), d.selected.contains(&c.path), n, cx));
                n += 1;
            }
        }
        let items = plan(&report, &d.selected);
        let sel: u64 = items.iter().map(|(a, _)| a.bytes).sum();
        let ready = sel > 0 && !d.running;
        let footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(20.))
            .py(px(10.))
            .border_t_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(div().flex_1().text_size(px(12.5)).text_color(t.ink_muted).child(i18n::tn!("app.dev.selected", items.len() as u64, size = human(sel))))
            .child(
                ui::filled_button(t, "dev-delete", t.danger)
                    .child(if d.running { i18n::t!("app.dev.deleting") } else { i18n::t!("app.dev.delete", size = human(sel)) })
                    .when(!ready, |b| b.opacity(0.4).cursor_default())
                    .when(ready, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            if let Some(d) = this.dev.as_mut() {
                                d.confirm = true;
                            }
                            cx.notify();
                        }))
                    }),
            );
        div().flex().flex_col().flex_1().min_h_0().child(list).child(footer).into_any_element()
    }

    /// First confirmation: every path and size, and how they go.
    fn render_dev_confirm(&self, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(DevState::Ready(report)) = self.dev.as_ref().map(|d| &d.state) else { return div().into_any_element() };
        let selected = self.dev.as_ref().map(|d| d.selected.clone()).unwrap_or_default();
        let items = plan(report, &selected);
        let sel: u64 = items.iter().map(|(a, _)| a.bytes).sum();
        let lines = items.iter().map(|(a, _)| format!("{:>9}  {}", human(a.bytes), a.path.display())).collect::<Vec<_>>().join("\n"); // i18n-ok: size and path
        let close = cx.listener(|this, _, _, cx| {
            if let Some(d) = this.dev.as_mut() {
                d.confirm = false;
            }
            cx.notify();
        });
        let card = div()
            .id("dev-confirm")
            .flex()
            .flex_col()
            .gap(px(12.))
            .w(px(640.))
            .max_h(relative(0.85))
            .p(px(22.))
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(i18n::tn!("app.dev.confirm_title", items.len() as u64, size = human(sel))))
            .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(t.ink_muted).child(i18n::t!("app.dev.confirm_how")))
            .child(div().id("dev-confirm-paths").flex_1().min_h(px(80.)).max_h(px(320.)).overflow_y_scroll().p(px(8.)).rounded(px(6.)).bg(t.canvas).text_size(px(11.)).line_height(px(15.)).font_family("Menlo").child(lines)) // i18n-ok: font
            .child(div().text_size(px(12.)).line_height(px(17.)).text_color(t.ink_muted).child(i18n::t!("app.dev.confirm_safe")))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(ui::outline_button(t, "dev-confirm-cancel").child(i18n::t!("common.cancel")).on_click(close))
                    .child(ui::filled_button(t, "dev-confirm-ok", t.danger).child(i18n::t!("app.dev.confirm_ok", size = human(sel))).on_click(cx.listener(|this, _, window, cx| this.dev_delete(window, cx)))),
            );
        let close_bg = cx.listener(|this, _, _, cx| {
            if let Some(d) = this.dev.as_mut() {
                d.confirm = false;
            }
            cx.notify();
        });
        div().absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(if t.dark { 0.55 } else { 0.32 })).on_mouse_down(MouseButton::Left, close_bg).child(card).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Artifact, ArtifactKind, Project, Report, default_selection, parse_roots, plan};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn art(p: &str, kind: ArtifactKind, bytes: u64, modified: i64) -> Artifact {
        Artifact { path: PathBuf::from(p), kind, bytes, files: 1, modified }
    }

    #[test]
    fn selection_and_plan_are_stable() {
        let now = 1_800_000_000;
        let old = now - 30 * 86_400;
        let r = Report {
            projects: vec![
                Project { root: "/c/old".into(), git: true, worktree: false, modified: old, artifacts: vec![art("/c/old/target", ArtifactKind::Target, 10, old), art("/c/old/node_modules", ArtifactKind::NodeModules, 5, old)], bytes: 15 },
                Project { root: "/c/new".into(), git: true, worktree: true, modified: now - 3600, artifacts: vec![art("/c/new/target", ArtifactKind::Target, 50, now)], bytes: 50 },
            ],
            caches: vec![art("/h/.cache/x-target", ArtifactKind::StandaloneTarget, 40, old), art("/h/.cargo/registry", ArtifactKind::CargoRegistry, 30, old), art("/h/.cache/y-target", ArtifactKind::StandaloneTarget, 40, now)],
        };
        let sel = default_selection(&r, now);
        let want: HashSet<PathBuf> = ["/c/old/target", "/c/old/node_modules", "/h/.cache/x-target"].iter().map(PathBuf::from).collect();
        assert_eq!(sel, want, "recent projects and download caches are not pre-selected");
        let p = plan(&r, &sel);
        let order: Vec<&Path> = p.iter().map(|(a, _)| a.path.as_path()).collect();
        assert_eq!(order, [Path::new("/c/old/target"), Path::new("/c/old/node_modules"), Path::new("/h/.cache/x-target")]);
        assert_eq!(p[0].1.map(|p| p.root.as_path()), Some(Path::new("/c/old")));
        assert!(p[2].1.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn roots_parse() {
        let home = PathBuf::from("/Users/ana");
        assert_eq!(parse_roots(" ~/code, /Volumes/Work\n\n~ ,relative", &home), [home.join("code"), PathBuf::from("/Volumes/Work"), home.clone()]);
        assert!(parse_roots("", &home).is_empty());
    }
}
