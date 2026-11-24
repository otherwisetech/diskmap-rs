//! The AI panel (otherwise-agent-panel), docked next to every screen and toggled with ⌘J: the
//! user's own Claude Code, Codex, opencode or Ollama with `diskmap mcp` attached for that session.
//!
//! Once a session asks for it, the window keeps a snapshot in its own private folder: the scan,
//! pruned (`scan::snapshot`), and the Developer projects report with git state (searched for the
//! assistant if the Dev view hasn't). The server reads only that. Outside "Solo lectura" it can
//! also propose moving cleanup candidates to the Trash: each proposal is an Aplicar / Rechazar
//! card listing every folder; an accepted one is checked again (`scan::dev::delete_with`, the
//! `scan::safety` rules included) and goes to the Trash. Nothing is ever pre-selected.
use crate::app::{DiskApp, Phase};
use crate::theme::Tokens;
use gpui::*;
use otherwise_agent_panel::ui::{AgentHost, AgentPanel, PanelStyle};
use otherwise_agent_panel::{McpServer, PendingChange, PermissionMode};
use rayon::prelude::*;
use scan::dev::{self, Artifact, GitState, Project, Report};
use scan::mcp::Proposal;
use scan::{Tree, human};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const INSTRUCTIONS: &str = "This is diskmap, a desktop app that shows what fills a disk: a treemap of one scanned folder and Developer projects (regenerable build folders and package caches by project). The MCP server \"diskmap\" reads the window's scan and projects report; quote its numbers, never estimate. Only cleanup_candidates may ever be suggested for removal; never suggest deleting tool settings or installs (hidden folders in the home folder: ~/.config, ~/.claude, ~/.codex, ~/.opencode, ~/.vscode-server, ~/.cargo, ~/.nvm…), ~/Library, cloud folders, apps, whole user folders or a project itself. To remove, use propose_trash: it becomes a card the user accepts, and accepted folders go to the Trash; never claim something was removed. Answer in the user's language (usually Spanish), plainly, with sizes like diskmap shows them."; // i18n-ok: agent instructions (English)

/// Git state is read for at most this many repositories per search.
const MAX_GIT: usize = 200;

/// The assistant's side of the window: its private folder, the cards waiting, and the projects
/// report it shares.
pub struct Assistant {
    pub panel: Option<WeakEntity<AgentPanel>>,
    pub session: PathBuf,
    pub pending: HashMap<String, Proposal>,
    pub report: Option<Rc<Report>>,
    pub git: HashMap<PathBuf, GitState>,
    pub searching: bool,
    generation: u64,
    /// (tree rev, report generation) last written.
    written: Cell<Option<(u64, u64)>>,
}

impl Assistant {
    pub fn new() -> Assistant {
        Assistant { panel: None, session: make_session_dir(), pending: HashMap::new(), report: None, git: HashMap::new(), searching: false, generation: 0, written: Cell::new(None) }
    }

    fn snapshot_path(&self) -> PathBuf {
        self.session.join("snapshot.json")
    }
}

/// A private per-user folder for this window's assistant: the snapshot and the proposals inbox.
pub fn make_session_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("diskmap-assistant-{}", std::process::id()));
    let _ = std::fs::create_dir_all(dir.join("inbox"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

/// `diskmap mcp` for a session in `mode`: read-only, or proposing into the window's inbox.
pub fn mcp_server(exe: &Path, session: &Path, mode: PermissionMode) -> McpServer {
    let (snap, inbox) = (session.join("snapshot.json").display().to_string(), session.join("inbox").display().to_string());
    let lang = i18n::locale();
    let mut args = vec!["mcp", "--snapshot", snap.as_str(), "--lang", lang]; // i18n-ok: command line
    if mode != PermissionMode::ReadOnly {
        args.extend(["--propose", "--inbox", inbox.as_str()]); // i18n-ok: flags
    }
    McpServer::new(scan::mcp::NAME, exe, &args)
}

pub fn set_panel_locale(code: &str) {
    otherwise_agent_panel::i18n::set_locale(code);
    otherwise_agent_panel::i18n::override_label("panel.privacy.cli", i18n::t!("app.agent.privacy"));
    otherwise_agent_panel::i18n::override_label("panel.privacy.ollama_remote", i18n::t!("app.agent.privacy_ollama_remote"));
}

pub struct Host {
    pub app: WeakEntity<DiskApp>,
}

impl AgentHost for Host {
    fn app_id(&self) -> String {
        "diskmap".into() // i18n-ok: app id
    }

    fn app_name(&self) -> String {
        crate::APP_NAME.into()
    }

    fn mcp_servers(&self, mode: PermissionMode, cx: &App) -> Vec<McpServer> {
        let Some(app) = self.app.upgrade() else { return Vec::new() };
        let a = app.read(cx);
        a.write_snapshot();
        vec![mcp_server(&std::env::current_exe().unwrap_or_default(), &a.assistant.session, mode)]
    }

    fn context(&self, cx: &App) -> Option<String> {
        let a = self.app.upgrade()?;
        let a = a.read(cx);
        let screen = if a.dev.is_some() {
            "Developer projects"
        } else {
            match a.phase {
                Phase::Start => "start screen (nothing scanned)",
                Phase::Scanning { .. } => "scanning",
                Phase::Failed { .. } => "scan failed",
                Phase::Ready => "treemap",
            }
        }; // i18n-ok: context for the agent (English)
        let mut v = serde_json::json!({"screen": screen});
        if let Some(t) = &a.tree {
            v["scanned"] = serde_json::json!(t.root_path());
            v["folder_in_view"] = serde_json::json!(t.path(a.zoom));
            if let Some(s) = a.selected {
                v["selected"] = serde_json::json!(t.path(s));
            }
        }
        Some(v.to_string())
    }

    fn instructions(&self) -> Option<String> {
        Some(INSTRUCTIONS.into())
    }

    fn suggestions(&self) -> Vec<String> {
        vec![i18n::t!("app.agent.suggest_full"), i18n::t!("app.agent.suggest_safe"), i18n::t!("app.agent.suggest_old")]
    }

    fn resolve_change(&self, id: &str, approve: bool, window: &mut Window, cx: &mut App) -> Result<(), String> {
        self.app.update(cx, |a, cx| a.resolve_proposal(id, approve, window, cx)).map_err(|e| e.to_string())?
    }
}

pub fn style(t: &Tokens) -> PanelStyle {
    let soft = |mut c: Hsla| {
        c.a = 0.12;
        c
    };
    PanelStyle {
        background: t.canvas,
        surface: t.surface,
        raised: t.canvas,
        border: t.border,
        ink: t.ink,
        muted: t.ink_muted,
        accent: t.accent,
        accent_soft: t.accent_soft,
        danger: t.danger,
        danger_soft: soft(t.danger),
        ok: t.accent,
        note: t.warning,
        note_soft: soft(t.warning),
        terminal_bg: t.surface,
        terminal_fg: t.ink,
        mono_font: if cfg!(windows) { "Consolas".into() } else { "Menlo".into() }, // i18n-ok: font name
        dark: t.dark,
    }
}

/// The report's build folder or cache at `path` (and its project), if it still lists it.
pub fn find_item<'a>(r: &'a Report, path: &Path, project: Option<&Path>) -> Option<(&'a Artifact, Option<&'a Project>)> {
    match project {
        Some(root) => r.projects.iter().filter(|p| p.root == root).find_map(|p| p.artifacts.iter().find(|a| a.path == path).map(|a| (a, Some(p)))),
        None => r.caches.iter().find(|c| c.path == path).map(|c| (c, None)),
    }
}

/// Card text: how many folders and how much, then every path with its size, then why.
pub fn proposal_text(p: &Proposal) -> (String, String) {
    let title = i18n::tn!("app.card.trash", p.items.len() as u64, size = human(p.bytes()));
    let mut detail = p.items.iter().map(|i| format!("{} · {}", human(i.bytes), i.path.display())).collect::<Vec<_>>().join("\n"); // i18n-ok: size · path
    detail.push('\n');
    detail.push_str(&i18n::t!("app.card.trash_how"));
    if !p.why.is_empty() {
        detail.push('\n');
        detail.push_str(&p.why);
    }
    (title, detail)
}

impl DiskApp {
    /// A session has asked for the snapshot.
    pub fn assistant_active(&self) -> bool {
        self.assistant.snapshot_path().exists()
    }

    fn agent_panel(&self) -> Option<Entity<AgentPanel>> {
        self.assistant.panel.as_ref().and_then(|p| p.upgrade())
    }

    /// Keeps the assistant's snapshot current (rewritten only when the scan or report changed).
    pub fn write_snapshot(&self) {
        let a = &self.assistant;
        let key = (self.rev, a.generation);
        if a.written.get() == Some(key) && a.snapshot_path().exists() {
            return;
        }
        let scan = self.tree.as_ref().filter(|_| matches!(self.phase, Phase::Ready)).map(scan::snapshot::scan_json);
        let dev = a.report.as_ref().map(|r| scan::snapshot::dev_json(r, &a.git));
        let home = dev::canonical(&otherwise_link::paths::home()).unwrap_or_else(|_| otherwise_link::paths::home());
        let json = scan::snapshot::snapshot_json(&home, scan, dev, a.searching || a.report.is_none(), crate::dev::now());
        let tmp = a.session.join(".snapshot.tmp");
        if std::fs::write(&tmp, &json).is_ok() && std::fs::rename(&tmp, a.snapshot_path()).is_ok() {
            a.written.set(Some(key));
        }
    }

    /// Every second: the projects report and snapshot (once a session asked for them) and new
    /// proposals.
    pub fn assistant_tick(&mut self, cx: &mut Context<Self>) {
        if self.assistant_active() {
            if self.assistant.report.is_none() && !self.assistant.searching {
                self.assistant_search(cx);
            }
            self.write_snapshot();
        }
        let new = scan::mcp::take(&self.assistant.session.join("inbox"));
        if new.is_empty() {
            return;
        }
        let Some(panel) = self.agent_panel() else { return };
        for p in new {
            let (title, detail) = proposal_text(&p);
            let change = PendingChange::new(p.id.clone(), title, detail);
            self.assistant.pending.insert(p.id.clone(), p);
            panel.update(cx, |panel, cx| panel.report_pending_change(change, cx));
        }
    }

    /// Searches the Developer projects folders for the assistant (the Dev view's report is used
    /// instead when it has one).
    fn assistant_search(&mut self, cx: &mut Context<Self>) {
        if let Some(crate::dev::DevState::Ready(r)) = self.dev.as_ref().map(|d| &d.state) {
            let r = r.clone();
            self.assistant_take_report(r, cx);
            return;
        }
        self.assistant.searching = true;
        let roots = crate::dev::load_roots();
        let home = otherwise_link::paths::home();
        let task = cx.background_executor().spawn(async move { dev::find(&roots, Some(&home), &scan::Progress::default()) });
        cx.spawn(async move |this, cx| {
            let report = task.await;
            this.update(cx, |this, cx| this.assistant_take_report(Rc::new(report), cx)).ok();
        })
        .detach();
    }

    /// A fresh projects report (the assistant's search or the Dev view's): git state is read in
    /// the background, then the snapshot is rewritten.
    pub fn assistant_take_report(&mut self, report: Rc<Report>, cx: &mut Context<Self>) {
        self.assistant.searching = true;
        self.assistant.generation += 1;
        let generation = self.assistant.generation;
        let roots: Vec<PathBuf> = report.projects.iter().filter(|p| p.git).take(MAX_GIT).map(|p| p.root.clone()).collect();
        self.assistant.report = Some(report);
        let task = cx.background_executor().spawn(async move { roots.par_iter().map(|r| (r.clone(), dev::git_state(r))).collect::<HashMap<_, _>>() });
        cx.spawn(async move |this, cx| {
            let git = task.await;
            this.update(cx, |this, _| {
                if this.assistant.generation == generation {
                    this.assistant.git = git;
                    this.assistant.searching = false;
                    this.assistant.generation += 1;
                }
            })
            .ok();
        })
        .detach();
    }

    /// Aplicar / Rechazar on a card. Aplicar re-checks every folder and moves it to the Trash.
    pub fn resolve_proposal(&mut self, id: &str, approve: bool, _window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(p) = self.assistant.pending.remove(id) else {
            return if approve { Err(i18n::t!("app.card.gone")) } else { Ok(()) };
        };
        if !approve {
            return Ok(());
        }
        let report = self.assistant.report.clone().ok_or_else(|| i18n::t!("app.card.gone"))?;
        let mut items: Vec<(Artifact, Option<Project>)> = Vec::new();
        let mut skipped = 0usize;
        for i in &p.items {
            match find_item(&report, &i.path, i.project.as_deref()) {
                Some((a, pr)) => items.push((a.clone(), pr.cloned())),
                None => skipped += 1,
            }
        }
        if items.is_empty() {
            return Err(i18n::t!("app.card.gone"));
        }
        let home = otherwise_link::paths::home();
        let task = cx.background_executor().spawn(async move {
            let mut done: Vec<(PathBuf, u64)> = Vec::new();
            let mut left = 0usize;
            for (a, pr) in &items {
                match dev::delete_with(a, pr.as_ref(), Some(&home), |x| trash::delete(x).map_err(|e| e.to_string())) {
                    Ok(()) => done.push((a.path.clone(), a.bytes)),
                    Err(_) => left += 1,
                }
            }
            (done, left)
        });
        cx.spawn(async move |this, cx| {
            let (done, left) = task.await;
            this.update(cx, |this, cx| this.trashed_by_assistant(done, left + skipped, cx)).ok();
        })
        .detach();
        Ok(())
    }

    fn trashed_by_assistant(&mut self, done: Vec<(PathBuf, u64)>, left: usize, cx: &mut Context<Self>) {
        let bytes: u64 = done.iter().map(|(_, b)| b).sum();
        if let Some(tree) = self.tree.as_mut() {
            for (p, _) in &done {
                if let Some(id) = tree.find(p).filter(|id| *id != Tree::ROOT) {
                    if tree.is_ancestor(id, self.zoom) {
                        self.zoom = tree.node(id).parent.unwrap_or(Tree::ROOT);
                    }
                    if self.selected.is_some_and(|s| tree.is_ancestor(id, s)) {
                        self.selected = None;
                    }
                    if self.search_root.is_some_and(|r| tree.is_ancestor(id, r)) {
                        self.search_root = None;
                    }
                    tree.remove(id);
                }
            }
            self.hovered = None;
            self.rev += 1;
        }
        // searched again on the next tick, so the assistant sees what's left
        self.assistant.report = None;
        self.assistant.generation += 1;
        if self.dev.is_some() {
            self.scan_dev(cx);
        }
        let text = if left == 0 { i18n::tn!("app.card.trashed", done.len() as u64, size = human(bytes)) } else { i18n::tn!("app.card.trashed_some", done.len() as u64, size = human(bytes), left = left) };
        self.toast(text, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionMode, find_item, mcp_server, proposal_text};
    use scan::dev::{Artifact, ArtifactKind, Project, Report};
    use scan::mcp::{Item, Proposal};
    use std::path::{Path, PathBuf};

    #[test]
    fn read_only_unless_the_mode_allows_proposals() {
        let exe = Path::new("/Applications/diskmap.app/Contents/MacOS/diskmap");
        let s = Path::new("/tmp/s");
        let ro = mcp_server(exe, s, PermissionMode::ReadOnly).args;
        assert_eq!(&ro[..3], ["mcp", "--snapshot", "/tmp/s/snapshot.json"]);
        assert!(!ro.iter().any(|a| a == "--propose"));
        assert!(mcp_server(exe, s, PermissionMode::Ask).args.ends_with(&["--propose".to_string(), "--inbox".into(), "/tmp/s/inbox".into()]));
        assert_eq!(mcp_server(exe, s, PermissionMode::Ask).name, "diskmap");
    }

    #[test]
    fn approved_items_must_still_be_in_the_report() {
        let a = Artifact { path: "/h/code/app/target".into(), kind: ArtifactKind::Target, bytes: 10, files: 1, modified: 0 };
        let r = Report { projects: vec![Project { root: "/h/code/app".into(), git: true, worktree: false, modified: 0, artifacts: vec![a.clone()], bytes: 10 }], caches: vec![] };
        assert!(find_item(&r, &a.path, Some(Path::new("/h/code/app"))).is_some());
        assert!(find_item(&r, &a.path, Some(Path::new("/h/code"))).is_none(), "another project");
        assert!(find_item(&r, &a.path, None).is_none(), "not a home cache");
        assert!(find_item(&r, Path::new("/h/.config/opencode/node_modules"), Some(Path::new("/h/.config/opencode"))).is_none());
        let p = Proposal { id: "x".into(), items: vec![Item { path: PathBuf::from("/h/code/app/target"), kind: "target".into(), project: Some("/h/code/app".into()), bytes: 10 }], why: "viejo".into(), created_at: "0".into() };
        let (title, detail) = proposal_text(&p);
        assert!(!title.is_empty() && detail.contains("/h/code/app/target") && detail.ends_with("viejo"), "every path on the card");
    }
}
