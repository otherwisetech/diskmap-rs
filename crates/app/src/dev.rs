//! Developer projects: build folders (`target/`, `node_modules/`, `.next/`, Gradle, DerivedData)
//! and package caches found under some roots (`scan::dev::find`), grouped by project with their
//! last change, "Open in rama" for git repositories and the worktrees rama knows marked. Nothing
//! is pre-selected; each row says what it is and why it is safe. Cleanup moves only regenerable
//! folders to the Trash, after an in-app list of every path and size and a second, native
//! confirmation; `scan::dev::delete_with` re-checks each one (`scan::safety` exclusions included).
//! rama's worktrees report (otherwise-link SPEC §13) adds a "Worktrees" section: each worktree's
//! size, build folders and why rama would or wouldn't remove it. Its worktree folders are searched
//! too, so every build folder offered here comes from our own scan; rama's report never makes
//! anything deletable. Worktrees are removed in rama ("Free up space in rama…"), never here.
use crate::app::DiskApp;
use crate::theme::Tokens;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{Disableable as _, IconName};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputState};
use scan::dev::{self, Artifact, ArtifactKind, Project, Refusal, Report};
use scan::{Progress, human};
use otherwise_link::worktrees;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

pub enum DevState {
    Scanning { progress: Arc<Progress>, started: Instant },
    Ready(Rc<Report>),
}

pub struct DevView {
    pub roots: Entity<InputState>,
    pub state: DevState,
    pub selected: HashSet<PathBuf>,
    /// Projects whose build folders are folded away (all are shown by default).
    pub collapsed: HashSet<PathBuf>,
    /// Worktree folders rama has listed.
    pub known: HashSet<PathBuf>,
    pub rama: bool,
    pub confirm: bool,
    pub running: bool,
    /// (freed, left alone with why) of the last cleanup.
    pub result: Option<(u64, Vec<(PathBuf, String)>)>,
    /// rama's worktrees report on this computer (paths made canonical like our scan's).
    pub worktrees: Option<Rc<worktrees::Report>>,
    /// The repository a link from rama asked for: listed first and unfolded.
    pub focus: Option<PathBuf>,
    /// Repositories of the Worktrees section that are folded away.
    pub folded: HashSet<PathBuf>,
    generation: u64,
}

/// The folders searched: the user's roots plus rama's worktrees that none of them covers.
pub fn with_rama_roots(roots: &[PathBuf], rama: Option<&worktrees::Report>, home: &Path) -> Vec<PathBuf> {
    let mut out = roots.to_vec();
    for w in rama.map(|r| r.paths()).unwrap_or_default() {
        if w.is_absolute() && !out.iter().any(|r| w.starts_with(r)) && !scan::safety::protected_tree(&w, Some(home)) {
            out.push(w);
        }
    }
    out
}

/// The report with every path as our scan spells it (`dev::canonical`), where it still exists.
pub fn canonical_report(mut r: worktrees::Report) -> worktrees::Report {
    let c = |p: &mut PathBuf| {
        if let Ok(x) = dev::canonical(p) {
            *p = x;
        }
    };
    for repo in &mut r.repos {
        c(&mut repo.root);
        for w in &mut repo.worktrees {
            c(&mut w.path);
            for k in &mut w.caches {
                c(&mut k.path);
            }
        }
    }
    r
}

/// Our own scan's build folders in the worktree at `wt` (its project).
pub fn worktree_folders<'a>(r: &'a Report, wt: &Path) -> Vec<&'a Artifact> {
    r.projects.iter().filter(|p| p.root == wt).flat_map(|p| p.artifacts.iter()).collect()
}

/// "Select unused" for one repository: our build folders of the worktrees rama would remove,
/// and the ones rama offers itself now (idle long enough), when our scan found exactly that
/// folder. Never anything of a worktree with changes or unpushed work, never a path only rama
/// listed.
pub fn unused_in(r: &Report, repo: &worktrees::Repo) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for w in &repo.worktrees {
        if w.blockers.iter().any(|b| matches!(b, worktrees::Blocker::Changes | worktrees::Blocker::Unreadable)) {
            continue;
        }
        for a in worktree_folders(r, &w.path) {
            if w.removable || w.caches.iter().any(|c| c.offered && c.path == a.path) {
                out.push(a.path.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Repositories in the order shown: the one asked for first, then by name.
pub fn repo_order<'a>(rep: &'a worktrees::Report, focus: Option<&Path>) -> Vec<&'a worktrees::Repo> {
    let hit = |r: &worktrees::Repo| focus.is_some_and(|f| r.root == f || r.worktrees.iter().any(|w| w.path == f));
    let mut v: Vec<&worktrees::Repo> = rep.repos.iter().collect();
    v.sort_by(|a, b| hit(b).cmp(&hit(a)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.root.cmp(&b.root)));
    v
}

pub fn blocker_text(b: worktrees::Blocker) -> String {
    use worktrees::Blocker as B;
    match b {
        B::Main => i18n::t!("app.dev.wt.why_main"),
        B::Open => i18n::t!("app.dev.wt.why_open"),
        B::Locked => i18n::t!("app.dev.wt.why_locked"),
        B::Changes => i18n::t!("app.dev.wt.why_changes"),
        B::Unreadable => i18n::t!("app.dev.wt.why_unreadable"),
        B::Unpushed => i18n::t!("app.dev.wt.why_unpushed"),
        B::NotMerged => i18n::t!("app.dev.wt.why_not_merged"),
        B::ProtectedBranch => i18n::t!("app.dev.wt.why_protected_branch"),
        B::ProtectedPlace => i18n::t!("app.dev.wt.why_protected_place"),
        B::NotMeasured => i18n::t!("app.dev.wt.why_not_measured"),
        B::Other => i18n::t!("app.dev.wt.why_other"),
    }
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

/// What is ticked when a search finishes: nothing, whatever its age or size. The user picks.
pub fn default_selection(_: &Report) -> HashSet<PathBuf> {
    HashSet::new()
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

/// The row's category and what happens if it goes ("regenerated when…").
fn kind_why(k: ArtifactKind) -> String {
    let category = if k.is_download_cache() || k == ArtifactKind::DerivedData { i18n::t!("app.dev.cat_cache") } else { i18n::t!("app.dev.cat_build") };
    let why = i18n::t!(&format!("app.dev.why.{}", k.id()));
    format!("{category} · {why}") // i18n-ok: separator
}

fn age(t: i64) -> String {
    match days_since(t, now()) {
        i64::MAX => String::new(),
        0 => i18n::t!("app.dev.today"),
        d => i18n::tn!("app.dev.days_ago", d as u64),
    }
}

/// A link from rama names a repository the last search didn't have: search again.
fn repo_focus_changed(app: &DiskApp) -> bool {
    let Some(d) = app.dev.as_ref() else { return false };
    let Some(f) = d.focus.as_deref() else { return false };
    !d.worktrees.as_ref().is_some_and(|r| r.repos.iter().any(|x| x.root == f || x.worktrees.iter().any(|w| w.path == f)))
}

impl DiskApp {
    pub fn open_dev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dev.is_none() {
            let text = load_roots().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
            let roots = cx.new(|cx| InputState::new(window, cx).default_value(text));
            self.dev = Some(DevView { roots, state: DevState::Ready(Rc::new(Report::default())), selected: HashSet::new(), collapsed: HashSet::new(), known: HashSet::new(), rama: false, confirm: false, running: false, result: None, worktrees: None, focus: None, folded: HashSet::new(), generation: 0 });
            self.scan_dev(cx);
        }
        cx.notify();
    }

    /// Developer projects, with `repo`'s worktrees first (a link from rama).
    pub fn open_dev_at(&mut self, repo: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let fresh = self.dev.is_none();
        self.open_dev(window, cx);
        if let Some(d) = self.dev.as_mut() {
            d.focus = repo.map(|p| dev::canonical(&p).unwrap_or(p));
            if let Some(f) = &d.focus {
                d.folded.retain(|r| r != f);
            }
        }
        // the report may be newer than the last search
        if !fresh && repo_focus_changed(self) {
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
            let wts = crate::otherwise::rama_report(&otherwise_link::Root::system()).map(canonical_report);
            let report = dev::find(&with_rama_roots(&roots, wts.as_ref(), &home), Some(&home), &p);
            let mut known = crate::otherwise::rama_worktrees(&crate::otherwise::rama_worktrees_file());
            known.extend(wts.iter().flat_map(|r| r.paths()));
            let rama = crate::otherwise::rama_installed();
            (report, known, rama, wts)
        });
        cx.spawn(async move |this, cx| {
            let (report, known, rama, wts) = task.await;
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
                d.selected = default_selection(&report);
                d.known = known;
                d.rama = rama;
                d.worktrees = wts.map(Rc::new);
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
                        match dev::delete_with(a, p.as_ref(), Some(&home), |x| trash::delete(x).map_err(|e| e.to_string())) {
                            Ok(()) => freed += a.bytes,
                            Err(Refusal::NotArtifact) => skipped.push((a.path.clone(), i18n::t!("app.dev.skip_not_artifact"))),
                            Err(Refusal::Protected) => skipped.push((a.path.clone(), i18n::t!("app.dev.skip_protected"))),
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
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(t.ink_muted).child(i18n::t!("app.dev.subtitle")))
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
                .child(div().text_size(px(13.)).text_color(t.ink_muted).child(i18n::t!("app.dev.scanning_stats", dirs = crate::app::group(progress.dirs.load(Relaxed)), size = human(progress.bytes.load(Relaxed)), secs = format!("{:.0}", started.elapsed().as_secs_f32()))))
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
                note = note.child(div().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.dev.skipped", path = p.display(), why = why)));
            }
            list = list.child(note);
        }
        list = list.child(div().text_size(px(13.)).text_color(t.ink_muted).pb(px(6.)).child(i18n::tn!("app.dev.summary", report.projects.len() as u64, builds = human(builds), caches = human(caches))));
        if report.projects.is_empty() && report.caches.is_empty() {
            list = list.child(div().py(px(30.)).text_center().text_size(px(13.)).text_color(t.ink_muted).child(i18n::t!("app.dev.none")));
        }
        if let Some(w) = d.worktrees.clone().filter(|w| !w.repos.is_empty()) {
            list = list.child(self.render_worktrees(t, &report, &w, cx));
        }
        let row_artifact = |a: &Artifact, rel: String, on: bool, i: usize, cx: &mut Context<Self>| {
            let path = a.path.clone();
            let line = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(Checkbox::new(("dev-art", i)).checked(on).on_click(cx.listener(move |this, on: &bool, _, cx| this.dev_toggle(vec![path.clone()], *on, cx))))
                .child(div().flex_none().w(px(150.)).text_size(px(12.)).child(kind_label(a.kind)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(t.ink_muted).child(rel))
                .child(div().flex_none().w(px(110.)).text_size(px(12.)).text_color(t.ink_muted).child(age(a.modified)))
                .child(div().flex_none().w(px(80.)).text_right().text_size(px(13.)).child(human(a.bytes)));
            div().flex().flex_col().pl(px(28.)).py(px(2.)).child(line).child(div().pl(px(26.)).text_size(px(11.)).line_height(px(15.)).text_color(t.ink_muted).child(kind_why(a.kind)))
        };
        let mut n = 0usize;
        for (pi, p) in report.projects.iter().enumerate() {
            let paths: Vec<PathBuf> = p.artifacts.iter().map(|a| a.path.clone()).collect();
            let all = paths.iter().all(|x| d.selected.contains(x));
            let open = !d.collapsed.contains(&p.root);
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
                        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.dev.only_builds", path = p.root.display())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.dev.as_mut()
                                && !d.collapsed.remove(&toggle_root)
                            {
                                d.collapsed.insert(toggle_root.clone());
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_none().w(px(110.)).text_size(px(12.)).text_color(t.ink_muted).child(age(p.modified)));
            if d.rama && p.git {
                let r = root.clone();
                row = row.child(ui::outline_button(t, ("dev-rama", pi)).h(px(24.)).px(px(8.)).text_size(px(12.)).child(i18n::t!("app.dev.open_rama")).on_click(cx.listener(move |this, _, _, cx| this.open_rama(r.clone(), cx))));
            }
            let s = root.clone();
            row = row
                .child(ui::outline_button(t, ("dev-show", pi)).h(px(24.)).px(px(8.)).text_size(px(12.)).child(i18n::t!("app.dev.show")).on_click(cx.listener(move |this, _, window, cx| {
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
            list = list.child(div().pt(px(16.)).pb(px(4.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.dev.caches"))).child(div().pb(px(4.)).text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.dev.caches_hint")));
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
            .child(div().flex_1().text_size(px(13.)).text_color(t.ink_muted).child(i18n::tn!("app.dev.selected", items.len() as u64, size = human(sel))))
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

    fn open_rama_reclaim(&mut self, repo: PathBuf, link: Option<String>, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async move { crate::otherwise::open_rama_reclaim(&repo, link.as_deref()).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                this.update(cx, |this, cx| this.toast(i18n::t!("app.dev.rama_failed", detail = e), cx)).ok();
            }
        })
        .detach();
    }

    /// rama's worktrees, by repository: size, our build folders in each (selectable), and why
    /// rama would or wouldn't remove it. Removing happens in rama.
    fn render_worktrees(&self, t: &Tokens, report: &Report, w: &worktrees::Report, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.dev.as_ref() else { return div().into_any_element() };
        let small = |s: String| div().text_size(px(12.)).line_height(px(16.)).text_color(t.ink_muted).child(s);
        let chip = |text: String, c: Hsla| div().flex_none().px(px(6.)).rounded(px(5.)).bg(c.opacity(0.14)).text_color(c).text_size(px(10.5)).child(text);
        let when = chrono::DateTime::parse_from_rfc3339(&w.updated_at).map(|x| x.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(); // i18n-ok: date format
        let mut sec = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .mb(px(14.))
            .p(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(t.border)
            .child(div().flex().items_center().gap(px(8.)).child(icon(IconName::Folder, 15., t.brand)).child(div().flex_1().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(i18n::t!("app.dev.wt.title"))).child(small(i18n::t!("app.dev.wt.reported", when = when))))
            .child(small(i18n::t!("app.dev.wt.hint")));
        for (ri, repo) in repo_order(w, d.focus.as_deref()).into_iter().enumerate() {
            let open = !d.folded.contains(&repo.root);
            let unused = unused_in(report, repo);
            let toggle = repo.root.clone();
            let mut head = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(8.))
                .mt(px(4.))
                .border_t_1()
                .border_color(t.border)
                .child(
                    div()
                        .id(("wt-repo", ri))
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 13., t.ink_muted))
                        .child(div().flex_none().max_w(px(240.)).truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(repo.name.clone()))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(t.ink_muted).child(i18n::tn!("app.dev.wt.repo_line", repo.worktrees.len() as u64, size = human(repo.bytes()))))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.dev.as_mut()
                                && !d.folded.remove(&toggle)
                            {
                                d.folded.insert(toggle.clone());
                            }
                            cx.notify();
                        })),
                );
            if repo.reclaimable > 0 {
                head = head.child(chip(i18n::t!("app.dev.wt.rama_frees", size = human(repo.reclaimable)), t.accent));
            }
            if !unused.is_empty() {
                let paths = unused.clone();
                head = head.child(ui::outline_button(t, ("wt-unused", ri)).h(px(24.)).px(px(8.)).text_size(px(12.)).child(i18n::tn!("app.dev.wt.select_unused", unused.len() as u64)).on_click(cx.listener(move |this, _, _, cx| this.dev_toggle(paths.clone(), true, cx))));
            }
            if d.rama {
                let (r, link) = (repo.root.clone(), repo.link.clone());
                head = head.child(ui::outline_button(t, ("wt-reclaim", ri)).h(px(24.)).px(px(8.)).text_size(px(12.)).child(i18n::t!("app.dev.wt.reclaim_in_rama")).on_click(cx.listener(move |this, _, _, cx| this.open_rama_reclaim(r.clone(), link.clone(), cx))));
                let r = repo.root.clone();
                head = head.child(ui::outline_button(t, ("wt-rama", ri)).h(px(24.)).px(px(8.)).text_size(px(12.)).child(i18n::t!("app.dev.open_rama")).on_click(cx.listener(move |this, _, _, cx| this.open_rama(r.clone(), cx))));
            }
            sec = sec.child(head);
            if !open {
                continue;
            }
            for (wi, wt) in repo.worktrees.iter().enumerate() {
                let ours = worktree_folders(report, &wt.path);
                let paths: Vec<PathBuf> = ours.iter().map(|a| a.path.clone()).collect();
                let build: u64 = ours.iter().map(|a| a.bytes).sum();
                let all = !paths.is_empty() && paths.iter().all(|x| d.selected.contains(x));
                let name = wt.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| wt.path.display().to_string());
                let status: AnyElement = if wt.removable {
                    chip(i18n::t!("app.dev.wt.removable"), t.accent).into_any_element()
                } else {
                    div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(if wt.blockers.iter().any(|b| matches!(b, worktrees::Blocker::Changes | worktrees::Blocker::Unpushed | worktrees::Blocker::NotMerged)) { t.warn } else { t.ink_muted }).child(wt.blockers.iter().map(|b| blocker_text(*b)).collect::<Vec<_>>().join(", ")).into_any_element() // i18n-ok: separator
                };
                let id = ri * 10_000 + wi;
                let pick = paths.clone();
                let lead = Checkbox::new(("wt-sel", id)).checked(all).when(paths.is_empty(), |c| c.disabled(true)).on_click(cx.listener(move |this, on: &bool, _, cx| this.dev_toggle(pick.clone(), *on, cx)));
                sec = sec.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(20.))
                        .py(px(3.))
                        .child(lead)
                        .child(div().flex_none().max_w(px(200.)).truncate().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(name))
                        .when(wt.main, |x| x.child(chip(i18n::t!("app.dev.wt.main"), t.ink_muted)))
                        .when_some(wt.branch.clone(), |x, b| x.child(div().flex_none().max_w(px(160.)).truncate().text_size(px(11.)).font_family("Menlo").text_color(t.ink_muted).child(b))) // i18n-ok: font
                        .child(status)
                        .child(div().flex_1())
                        .child(div().flex_none().w(px(140.)).text_right().text_size(px(12.)).text_color(t.ink_muted).child(if paths.is_empty() { i18n::t!("app.dev.wt.no_builds") } else { i18n::t!("app.dev.wt.builds", size = human(build)) }))
                        .child(div().flex_none().w(px(80.)).text_right().text_size(px(13.)).child(wt.bytes.map(human).unwrap_or_else(|| "—".into()))), // i18n-ok: placeholder
                );
            }
        }
        sec.into_any_element()
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
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(t.ink_muted).child(i18n::t!("app.dev.confirm_how")))
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
    use super::{Artifact, ArtifactKind, Project, Report, canonical_report, default_selection, parse_roots, plan, repo_order, unused_in, with_rama_roots, worktree_folders, worktrees};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn art(p: &str, kind: ArtifactKind, bytes: u64, modified: i64) -> Artifact {
        Artifact { path: PathBuf::from(p), kind, bytes, files: 1, modified }
    }

    #[test]
    fn nothing_is_pre_selected_and_plan_is_stable() {
        let now = 1_800_000_000;
        let old = now - 400 * 86_400;
        let r = Report {
            projects: vec![
                Project { root: "/c/old".into(), git: true, worktree: false, modified: old, artifacts: vec![art("/c/old/target", ArtifactKind::Target, 10, old), art("/c/old/node_modules", ArtifactKind::NodeModules, 5, old)], bytes: 15 },
                Project { root: "/c/new".into(), git: true, worktree: true, modified: now - 3600, artifacts: vec![art("/c/new/target", ArtifactKind::Target, 50, now)], bytes: 50 },
            ],
            caches: vec![art("/h/.cache/x-target", ArtifactKind::StandaloneTarget, 40, old), art("/h/.npm/_cacache", ArtifactKind::NpmCache, 30, old)],
        };
        assert!(default_selection(&r).is_empty(), "the user ticks every folder; none is pre-selected, however old or large");
        assert!(plan(&r, &default_selection(&r)).is_empty());
        let sel: HashSet<PathBuf> = ["/c/old/target", "/c/old/node_modules", "/h/.cache/x-target"].iter().map(PathBuf::from).collect();
        let p = plan(&r, &sel);
        let order: Vec<&Path> = p.iter().map(|(a, _)| a.path.as_path()).collect();
        assert_eq!(order, [Path::new("/c/old/target"), Path::new("/c/old/node_modules"), Path::new("/h/.cache/x-target")]);
        assert_eq!(p[0].1.map(|p| p.root.as_path()), Some(Path::new("/c/old")));
        assert!(p[2].1.is_none());
    }

    fn wt(path: &str, removable: bool, blockers: Vec<worktrees::Blocker>, caches: Vec<(&str, bool)>) -> worktrees::Worktree {
        worktrees::Worktree {
            path: PathBuf::from(path),
            branch: Some("b".into()),
            main: false,
            bytes: Some(100),
            touched: None,
            removable,
            blockers,
            caches: caches.into_iter().map(|(p, offered)| worktrees::Cache { path: PathBuf::from(p), kind: worktrees::CacheKind::Target, bytes: 10, offered }).collect(),
        }
    }

    fn rrepo(root: &str, name: &str, wts: Vec<worktrees::Worktree>) -> worktrees::Repo {
        worktrees::Repo { root: PathBuf::from(root), name: name.into(), updated_at: String::new(), default_branch: None, reclaimable: 0, link: None, worktrees: wts, shared: vec![] }
    }

    #[test]
    fn unused_takes_only_our_folders_of_worktrees_rama_would_clean() {
        use worktrees::Blocker as B;
        let proj = |root: &str, arts: Vec<Artifact>| Project { root: root.into(), git: true, worktree: true, modified: 0, bytes: arts.iter().map(|a| a.bytes).sum(), artifacts: arts };
        let r = Report {
            projects: vec![
                proj("/w/done", vec![art("/w/done/target", ArtifactKind::Target, 5, 0), art("/w/done/node_modules", ArtifactKind::NodeModules, 5, 0)]),
                proj("/w/idle", vec![art("/w/idle/target", ArtifactKind::Target, 5, 0), art("/w/idle/node_modules", ArtifactKind::NodeModules, 5, 0)]),
                proj("/w/dirty", vec![art("/w/dirty/target", ArtifactKind::Target, 5, 0)]),
                proj("/w/busy", vec![art("/w/busy/target", ArtifactKind::Target, 5, 0)]),
            ],
            caches: vec![],
        };
        let repo = rrepo(
            "/w/main",
            "shop",
            vec![
                // rama would remove it: all our folders in it
                wt("/w/done", true, vec![], vec![]),
                // unpushed, but rama offers its idle target: only that one
                wt("/w/idle", false, vec![B::Unpushed], vec![("/w/idle/target", true), ("/w/idle/node_modules", false)]),
                // changes: never, even if the report says offered
                wt("/w/dirty", false, vec![B::Changes], vec![("/w/dirty/target", true)]),
                // offered by rama but a path our scan doesn't have: not selectable
                wt("/w/busy", false, vec![B::NotMerged], vec![("/w/busy/other/target", true)]),
                // rama lists it, our scan found nothing there
                wt("/w/gone", true, vec![], vec![("/w/gone/target", true)]),
            ],
        );
        let got = unused_in(&r, &repo);
        assert_eq!(got, [PathBuf::from("/w/done/node_modules"), PathBuf::from("/w/done/target"), PathBuf::from("/w/idle/target")]);
        assert!(default_selection(&r).is_empty(), "the report selects nothing by itself");
        assert_eq!(worktree_folders(&r, Path::new("/w/idle")).len(), 2);
    }

    #[test]
    fn rama_worktrees_are_searched_and_the_asked_repo_comes_first() {
        let mut rep = worktrees::Report::new("rama", "h");
        rep.repos = vec![rrepo("/w/zeta", "zeta", vec![wt("/w/zeta", false, vec![], vec![])]), rrepo("/w/alpha", "Alpha", vec![wt("/w/alpha", false, vec![], vec![]), wt("/elsewhere/alpha-fix", false, vec![], vec![])])];
        let names = |f: Option<&Path>| repo_order(&rep, f).iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(None), ["Alpha", "zeta"]);
        assert_eq!(names(Some(Path::new("/w/zeta"))), ["zeta", "Alpha"]);
        assert_eq!(names(Some(Path::new("/elsewhere/alpha-fix"))), ["Alpha", "zeta"]);
        let home = PathBuf::from("/home/ana");
        let roots = with_rama_roots(&[PathBuf::from("/w")], Some(&rep), &home);
        assert_eq!(roots, [PathBuf::from("/w"), PathBuf::from("/elsewhere/alpha-fix")], "only worktrees no root covers are added");
        assert_eq!(with_rama_roots(&[PathBuf::from("/w")], None, &home), [PathBuf::from("/w")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_worktree_outside_the_roots_gets_its_build_folders_from_our_scan() {
        let dir = tempfile::tempdir().unwrap();
        let base = scan::dev::canonical(dir.path()).unwrap();
        let (roots, wt_dir) = (base.join("code"), base.join("elsewhere").join("shop-fix"));
        std::fs::create_dir_all(&roots).unwrap();
        std::fs::create_dir_all(wt_dir.join("target").join("debug")).unwrap();
        std::fs::write(wt_dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(wt_dir.join(".git"), "gitdir: /nowhere\n").unwrap();
        std::fs::write(wt_dir.join("target").join("debug").join("x"), vec![0u8; 4096]).unwrap();
        let mut rep = worktrees::Report::new("rama", "h");
        rep.repos = vec![rrepo(base.join("main").to_str().unwrap(), "shop", vec![wt(wt_dir.to_str().unwrap(), true, vec![], vec![])])];
        let rep = canonical_report(rep);
        // the temp folder plays the home folder (searching outside a home applies the system rules)
        let found = scan::dev::find(&with_rama_roots(&[roots], Some(&rep), &base), Some(&base), &scan::Progress::default());
        assert_eq!(worktree_folders(&found, &wt_dir).iter().map(|a| a.path.clone()).collect::<Vec<_>>(), [wt_dir.join("target")]);
        assert_eq!(unused_in(&found, &rep.repos[0]), [wt_dir.join("target")]);
    }

    #[cfg(unix)]
    #[test]
    fn roots_parse() {
        let home = PathBuf::from("/Users/ana");
        assert_eq!(parse_roots(" ~/code, /Volumes/Work\n\n~ ,relative", &home), [home.join("code"), PathBuf::from("/Volumes/Work"), home.clone()]);
        assert!(parse_roots("", &home).is_empty());
    }
}
