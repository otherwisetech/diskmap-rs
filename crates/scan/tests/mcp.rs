//! The AI panel's MCP server over synthetic snapshots: a fake home in a temp dir, a scan of it,
//! and Developer projects reports — including a deliberately wrong one that lists tool configs,
//! installs, cloud folders and project roots — so we can prove they are never proposed.
use scan::dev::{self, Artifact, ArtifactKind, GitState, Project, Report};
use scan::mcp::{self, Server};
use scan::{Options, Progress, snapshot};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

/// Places the report below may name: the real projects and, as a wrong report would, places
/// that must never be proposed.
const NEVER: &[&str] = &[
    ".config/opencode/node_modules",
    ".opencode/node_modules",
    ".vscode-server/cli/servers/Stable-abc/server/node_modules",
    ".serverless/releases/4.17.1/package/node_modules",
    ".claude/node_modules",
    ".codex/target",
    ".cargo/registry",
    ".nvm/versions/node/v20/lib/node_modules",
    "Library/Application Support/Code/node_modules",
    "Library/Mobile Documents/com~apple~CloudDocs/app/node_modules",
    "Library/CloudStorage/OneDrive-Acme/app/target",
    "OneDrive - Acme/app/node_modules",
    "Applications/Tool.app/Contents/Resources/app/node_modules",
    "code/app",
    "Documents",
    "Downloads",
    "node_modules",
];

struct Fixture {
    _d: tempfile::TempDir,
    home: PathBuf,
    session: PathBuf,
}

impl Fixture {
    fn snap(&self) -> PathBuf {
        self.session.join("snapshot.json")
    }
    fn inbox(&self) -> PathBuf {
        self.session.join("inbox")
    }
}

fn art(path: PathBuf, kind: ArtifactKind, bytes: u64) -> Artifact {
    Artifact { path, kind, bytes, files: 3, modified: 1_600_000_000 }
}

/// A fake home with two real projects, a cache, and every NEVER place on disk; a scan of it and a
/// report that (wrongly) lists the NEVER places next to the real build folders.
fn fixture() -> Fixture {
    let d = tempfile::tempdir().unwrap();
    let home = dev::canonical(d.path()).unwrap().join("home");
    for n in NEVER {
        write(&home.join(n).join("x.bin"), 4_000);
    }
    write(&home.join("package.json"), 10);
    write(&home.join("code/app/Cargo.toml"), 10);
    fs::create_dir_all(home.join("code/app/.git")).unwrap();
    write(&home.join("code/app/target/debug/big"), 300_000);
    write(&home.join("Downloads/27-shaders/package.json"), 10);
    write(&home.join("Downloads/27-shaders/node_modules/x/i.js"), 80_000);
    write(&home.join(".npm/_cacache/index/x"), 50_000);
    write(&home.join("Movies/trip.mov"), 900_000);
    let session = dev::canonical(d.path()).unwrap().join("session");
    fs::create_dir_all(session.join("inbox")).unwrap();

    let tree = scan::scan(&home, &Options::default(), &Progress::default()).unwrap();
    let mut projects = vec![
        Project { root: home.join("code/app"), git: true, worktree: false, modified: 1_600_000_000, artifacts: vec![art(home.join("code/app/target"), ArtifactKind::Target, 300_000)], bytes: 300_000 },
        Project { root: home.join("Downloads/27-shaders"), git: false, worktree: false, modified: 1_600_000_000, artifacts: vec![art(home.join("Downloads/27-shaders/node_modules"), ArtifactKind::NodeModules, 80_000)], bytes: 80_000 },
    ];
    // what a careless search would have reported
    for n in NEVER {
        let p = home.join(n);
        let owner = p.parent().unwrap().to_path_buf();
        projects.push(Project { root: owner, git: false, worktree: false, modified: 1_500_000_000, artifacts: vec![art(p, ArtifactKind::NodeModules, 9_000_000)], bytes: 9_000_000 });
    }
    let report = Report { projects, caches: vec![art(home.join(".npm/_cacache"), ArtifactKind::NpmCache, 50_000), art(home.join(".cargo/registry"), ArtifactKind::NpmCache, 70_000)] };
    let mut git = HashMap::new();
    git.insert(home.join("code/app"), GitState { dirty: Some(true), unpushed: Some(false) });
    let s = snapshot::snapshot_json(&home, Some(snapshot::scan_json(&tree)), Some(snapshot::dev_json(&report, &git)), false, 1_700_000_000);
    fs::write(session.join("snapshot.json"), s).unwrap();
    Fixture { _d: d, home, session }
}

fn call(srv: &mut Server, name: &str, args: Value) -> Value {
    srv.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap()["result"].clone()
}

fn paths(v: &Value, list: &str) -> Vec<String> {
    v[list].as_array().unwrap().iter().map(|x| x["path"].as_str().unwrap().to_string()).collect()
}

#[test]
fn read_only_unless_proposals_are_on() {
    let f = fixture();
    let mut srv = Server::new(f.snap(), None);
    let init = srv.handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}})).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    let list = srv.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().all(|t| t["annotations"]["readOnlyHint"] == true && t["name"] != "propose_trash"));
    let refused = srv.handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "propose_trash", "arguments": {"paths": ["/x"]}}})).unwrap();
    assert_eq!(refused["error"]["code"], -32602);
    assert_eq!(Server::from_args(&["--snapshot".into(), "/s".into(), "--inbox".into(), "/i".into()]).unwrap().inbox, None, "an inbox without --propose stays read-only");
    assert!(Server::from_args(&["--snapshot".into(), "/s".into(), "--propose".into(), "--inbox".into(), "/i".into()]).unwrap().inbox.is_some());
    assert!(Server::from_args(&["--bogus".into()]).is_err());
}

#[test]
fn read_tools_answer_from_the_snapshot() {
    let f = fixture();
    let mut srv = Server::new(f.snap(), None);
    let o = call(&mut srv, "overview", json!({}))["structuredContent"].clone();
    assert_eq!(o["scanned"], json!(f.home));
    assert!(o["bytes"].as_u64().unwrap() >= 1_300_000);
    assert!(o["by_type"].as_array().unwrap().iter().any(|k| k["kind"] == "video"));
    let files = call(&mut srv, "biggest", json!({"what": "files", "limit": 1}))["structuredContent"].clone();
    assert_eq!(paths(&files, "files"), [f.home.join("Movies/trip.mov").display().to_string()]);
    assert_eq!(files["files"][0]["can_propose"], false, "personal files are never proposable");
    let under = call(&mut srv, "biggest", json!({"under": f.home.join("code").display().to_string()}))["structuredContent"].clone();
    assert!(paths(&under, "folders").iter().all(|p| p.starts_with(&f.home.join("code").display().to_string())));
    let types = call(&mut srv, "space_by_type", json!({}))["structuredContent"].clone();
    assert_eq!(types["by_type"][0]["kind"], "video");
    let dev = call(&mut srv, "dev_projects", json!({"sort": "idle"}))["structuredContent"].clone();
    let app = dev["projects"].as_array().unwrap().iter().find(|p| p["root"] == json!(f.home.join("code/app"))).unwrap();
    assert_eq!(app["uncommitted_changes"], true);
    assert_eq!(app["unpushed_commits"], false);
    assert!(app["idle_days"].as_i64().unwrap() > 365);
    assert_eq!(call(&mut srv, "biggest", json!({"top": 3}))["isError"], true, "unknown arguments are refused");
}

#[test]
fn excluded_paths_are_never_candidates_nor_proposed() {
    let f = fixture();
    let mut srv = Server::new(f.snap(), Some(f.inbox()));
    let c = call(&mut srv, "cleanup_candidates", json!({}))["structuredContent"].clone();
    let mut got = paths(&c, "candidates");
    got.sort();
    let mut want = [f.home.join("code/app/target"), f.home.join("Downloads/27-shaders/node_modules"), f.home.join(".npm/_cacache")].iter().map(|p| p.display().to_string()).collect::<Vec<_>>();
    want.sort();
    assert_eq!(got, want, "only build folders inside projects and exact caches");
    for cand in c["candidates"].as_array().unwrap() {
        assert!(!cand["regenerated"].as_str().unwrap().is_empty() && !cand["category"].as_str().unwrap().is_empty());
    }
    // the biggest folders include the excluded ones; none is proposable
    let big = call(&mut srv, "biggest", json!({"limit": 100}))["structuredContent"].clone();
    for fo in big["folders"].as_array().unwrap() {
        let p = fo["path"].as_str().unwrap();
        let rel = Path::new(p).strip_prefix(&f.home).unwrap();
        let real = ["code/app/target", "Downloads/27-shaders/node_modules", ".npm/_cacache"].iter().any(|g| rel == Path::new(g));
        if !real && (NEVER.iter().any(|n| rel.starts_with(n)) || rel == Path::new(".cargo/registry")) {
            assert_eq!(fo["can_propose"], false, "{p}");
        }
    }
    let dev = call(&mut srv, "dev_projects", json!({"limit": 200}))["structuredContent"].clone();
    for p in dev["projects"].as_array().unwrap() {
        for b in p["build_folders"].as_array().unwrap() {
            let rel = PathBuf::from(b["path"].as_str().unwrap()).strip_prefix(&f.home).unwrap().to_path_buf();
            assert_eq!(b["can_propose"], json!(!NEVER.iter().any(|n| rel == Path::new(n))), "{}", rel.display());
        }
    }
    // each excluded path, alone or mixed with a good one, is refused and nothing reaches the inbox
    let good = f.home.join("code/app/target").display().to_string();
    for n in NEVER.iter().chain([&".cargo/registry"]) {
        let bad = f.home.join(n).display().to_string();
        for args in [json!({"paths": [bad]}), json!({"paths": [good, bad], "why": "x"})] {
            let r = call(&mut srv, "propose_trash", args);
            assert_eq!(r["isError"], true, "{n} must be refused");
        }
    }
    let home_s = f.home.display().to_string();
    for p in ["/", "relative/target", "/System/Library", home_s.as_str()] {
        assert_eq!(call(&mut srv, "propose_trash", json!({"paths": [p]}))["isError"], true, "{p}");
    }
    assert!(mcp::take(&f.inbox()).is_empty(), "no refused proposal reaches the window");

    // the candidates go through as one card, nothing pre-chosen beyond what was asked
    let r = call(&mut srv, "propose_trash", json!({"paths": [good, good], "why": "Proyecto viejo"}))["structuredContent"].clone();
    assert_eq!(r["status"], "awaiting_approval");
    assert_eq!(r["folders"], 1, "duplicates folded");
    let props = mcp::take(&f.inbox());
    assert_eq!(props.len(), 1);
    assert_eq!(props[0].items.len(), 1);
    assert_eq!(props[0].items[0].path, f.home.join("code/app/target"));
    assert_eq!(props[0].items[0].kind, "target");
    assert_eq!(props[0].items[0].project.as_deref(), Some(f.home.join("code/app").as_path()));
    assert_eq!(props[0].why, "Proyecto viejo");
    assert!(f.home.join("code/app/target/debug/big").exists(), "the server never touches the disk");
    // a cache has no project
    call(&mut srv, "propose_trash", json!({"paths": [f.home.join(".npm/_cacache").display().to_string()]}));
    assert_eq!(mcp::take(&f.inbox())[0].items[0].project, None);
    // too many paths
    let many: Vec<String> = (0..=mcp::MAX_PROPOSAL).map(|i| format!("/x/{i}")).collect();
    assert_eq!(call(&mut srv, "propose_trash", json!({"paths": many}))["isError"], true);
}

#[test]
fn without_a_scan_or_report_the_agent_is_told() {
    let d = tempfile::tempdir().unwrap();
    let snap = d.path().join("snapshot.json");
    fs::write(&snap, snapshot::snapshot_json(d.path(), None, None, true, 1_700_000_000)).unwrap();
    let mut srv = Server::new(snap.clone(), Some(d.path().to_path_buf()));
    let o = call(&mut srv, "overview", json!({}))["structuredContent"].clone();
    assert!(o["scanned"].is_null() && o["dev_projects"]["searching"] == true);
    assert_eq!(call(&mut srv, "biggest", json!({}))["isError"], true);
    assert_eq!(call(&mut srv, "cleanup_candidates", json!({}))["isError"], true);
    let mut gone = Server::new(d.path().join("missing.json"), None);
    assert_eq!(call(&mut gone, "overview", json!({}))["isError"], true);
}
