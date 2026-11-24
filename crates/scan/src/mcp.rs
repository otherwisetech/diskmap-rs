//! `diskmap mcp --snapshot <file> [--propose --inbox <dir>]`: a small stdio MCP server (JSON-RPC
//! 2.0, one message per line) started by the app's AI panel for one session. It reads only the
//! snapshot the window keeps in its private session folder ([`crate::snapshot`]) and never
//! writes or deletes anything. With an inbox it also offers `propose_trash`: only build folders
//! and caches from the Developer projects report that [`crate::safety::protected`] allows; the
//! window shows each proposal as an Aplicar / Rechazar card, checks every folder again and moves
//! it to the Trash. Nothing is ever deleted for good, and nothing is chosen for the user.
//!
//! Every text here is for the agent (English); the user never sees it.
use crate::human;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const NAME: &str = "diskmap";
/// Most folders in one proposal.
pub const MAX_PROPOSAL: usize = 50;

const HELP: &str = "diskmap shows what fills a disk: a scan of one folder (sizes on disk, hard links counted once) and Developer projects, the regenerable build folders (target, node_modules, .next, Gradle, Xcode DerivedData) and package caches found under the user's folders, grouped by project with git state. Sizes are bytes; *_human fields are for quoting.

Pick the tool that answers the whole question in one call:
- what fills the disk, overall → overview
- the biggest folders or files, anywhere or under one folder → biggest
- space by file type (video, images, archives, code…) → space_by_type
- old projects and their build folders, with git state → dev_projects
- what can be removed safely → cleanup_candidates (the only things that may ever be proposed)

Safety rules, always: only cleanup_candidates are regenerable. Never suggest deleting tool settings or installs (hidden folders in the home folder such as ~/.config, ~/.claude, ~/.codex, ~/.opencode, ~/.vscode-server, ~/.cargo, ~/.nvm), anything in ~/Library, cloud folders (iCloud Drive, OneDrive, Dropbox), apps, Documents / Desktop / Downloads / Pictures as a whole, or a project folder itself; for big personal files, describe them and let the user decide in Finder. Quote diskmap's numbers; never estimate your own. The scan may be older than the disk: say so when it matters."; // i18n-ok: for agents

const PROPOSE_HELP: &str = "\n\npropose_trash changes nothing by itself: it becomes a card in diskmap that the user accepts (Aplicar) or rejects (Rechazar); the answer says awaiting_approval. Only paths from cleanup_candidates are accepted, nothing is pre-selected, and accepted folders go to the Trash (never a permanent delete) after diskmap checks each again. Propose only what the user asked for; never claim something was removed."; // i18n-ok: for agents

struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    propose: bool,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

const TOOLS: &[Tool] = &[
    Tool { name: "overview", description: "The whole picture in one call: the scanned folder, its total size and file count, the biggest folders right under it, space by type, and the Developer projects totals (build folders and caches, how many projects). Says when there is no scan yet or the projects search is still running.", schema: || obj(json!({}), &[]), propose: false }, // i18n-ok: for agents
    Tool { name: "biggest", description: "The biggest folders (default) or files in the scan, largest first, optionally only under one folder (`under`, an absolute path). Each says whether it may be proposed for the Trash (only build folders and caches from cleanup_candidates).", schema: || obj(json!({"what": {"type": "string", "enum": ["folders", "files"]}, "under": {"type": "string", "description": "absolute path inside the scan"}, "limit": {"type": "integer", "minimum": 1, "maximum": 100}}), &[]), propose: false }, // i18n-ok: for agents
    Tool { name: "space_by_type", description: "Space in the scan by file type (video, image, audio, archive, code, document, package = app bundles and libraries, other), with share of the total.", schema: || obj(json!({}), &[]), propose: false }, // i18n-ok: for agents
    Tool { name: "dev_projects", description: "Developer projects: each project folder with its build folders (kind, size, last build), days since anything else changed, and git state (git repository, worktree, uncommitted changes, commits not pushed; null = unknown). sort: size (default) or idle. min_idle_days keeps old projects only.", schema: || obj(json!({"sort": {"type": "string", "enum": ["size", "idle"]}, "min_idle_days": {"type": "integer", "minimum": 0}, "limit": {"type": "integer", "minimum": 1, "maximum": 200}}), &[]), propose: false }, // i18n-ok: for agents
    Tool { name: "cleanup_candidates", description: "Everything diskmap considers safe to remove: build folders inside projects and exact package caches, each with its category, what regenerates it, its project's age and git state, largest first. Nothing else may be proposed. min_idle_days keeps folders of projects untouched that long.", schema: || obj(json!({"min_idle_days": {"type": "integer", "minimum": 0}, "limit": {"type": "integer", "minimum": 1, "maximum": 200}}), &[]), propose: false }, // i18n-ok: for agents
    Tool { name: "propose_trash", description: "Propose moving folders to the Trash: `paths` exactly as cleanup_candidates gives them (at most 50). Shown to the user as one card listing every folder and its size; nothing moves unless they accept, and then only to the Trash. Anything else (settings, installs, cloud, personal files, a project itself) is refused.", schema: || obj(json!({"paths": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": MAX_PROPOSAL}, "why": {"type": "string", "description": "one short sentence for the user: why these"}}), &["paths"]), propose: true }, // i18n-ok: for agents
];

/// One folder of a proposal, as the window gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    /// [`crate::dev::ArtifactKind::id`].
    pub kind: String,
    /// The project it belongs to; `None` for a home cache.
    pub project: Option<PathBuf>,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub items: Vec<Item>,
    pub why: String,
    pub created_at: String,
}

impl Proposal {
    pub fn bytes(&self) -> u64 {
        self.items.iter().map(|i| i.bytes).sum()
    }

    fn to_json(&self) -> Value {
        json!({"id": self.id, "why": self.why, "created_at": self.created_at, "items": self.items.iter().map(|i| json!({"path": i.path, "kind": i.kind, "project": i.project, "bytes": i.bytes})).collect::<Vec<_>>()})
    }

    fn from_json(v: &Value) -> Option<Proposal> {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let items = v.get("items")?.as_array()?.iter().map(|i| Some(Item { path: PathBuf::from(s(i, "path")?), kind: s(i, "kind")?, project: s(i, "project").map(PathBuf::from), bytes: i.get("bytes")?.as_u64()? })).collect::<Option<Vec<_>>>()?;
        Some(Proposal { id: s(v, "id")?, items, why: s(v, "why").unwrap_or_default(), created_at: s(v, "created_at")? })
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn now_s() -> i64 {
    (now_ms() / 1000) as i64
}

/// Writes a proposal into `inbox` (atomic); the window that owns it picks it up.
pub fn write(inbox: &Path, items: Vec<Item>, why: &str) -> std::io::Result<Proposal> {
    if !inbox.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, inbox.display().to_string()));
    }
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let created_at = format!("{:020}-{seq:06}", now_ms());
    let p = Proposal { id: format!("trash-{created_at}-{}", std::process::id()), items, why: why.chars().take(300).collect(), created_at };
    let tmp = inbox.join(format!(".{}.tmp", p.id));
    std::fs::write(&tmp, p.to_json().to_string())?;
    std::fs::rename(&tmp, inbox.join(format!("{}.json", p.id)))?;
    Ok(p)
}

/// Takes every proposal out of `inbox`, oldest first; unreadable files are dropped.
pub fn take(inbox: &Path) -> Vec<Proposal> {
    let Ok(rd) = std::fs::read_dir(inbox) else { return vec![] };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let p = std::fs::read_to_string(e.path()).ok().filter(|s| s.len() < 256 << 10).and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| Proposal::from_json(&v));
        if std::fs::remove_file(e.path()).is_ok()
            && let Some(p) = p
        {
            out.push(p);
        }
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    out
}

/// What regenerates a build folder or cache, for the agent to tell the user.
pub fn regenerated_by(kind: &str) -> &'static str {
    match kind {
        "target" => "rebuilt by the next cargo build / mvn package",
        "node_modules" => "reinstalled from package.json by npm / pnpm / yarn install",
        "next" => "recreated by the next `next dev` or `next build`",
        "gradle" => "recreated the next time Gradle runs in the project",
        "gradle_build" => "rebuilt by the next gradle build",
        "derived_data" => "Xcode rebuilds it on the next build",
        "standalone_target" => "rebuilt by cargo builds that use this CARGO_TARGET_DIR",
        "npm_cache" => "npm downloads packages again when it needs them",
        "pnpm_store" => "pnpm downloads packages again on the next install",
        "gradle_cache" => "Gradle downloads dependencies again on the next build",
        _ => "regenerated by the tool that made it",
    } // i18n-ok: for agents
}

fn category(kind: &str) -> &'static str {
    match kind {
        "npm_cache" | "pnpm_store" | "gradle_cache" | "derived_data" => "regenerable cache",
        _ => "regenerable build output",
    } // i18n-ok: for agents
}

/// A build folder or cache the snapshot lists, with its project.
#[derive(Debug, Clone)]
struct Candidate {
    kind: String,
    bytes: u64,
    modified: i64,
    project: Option<Value>,
}

pub struct Server {
    pub snapshot: PathBuf,
    pub inbox: Option<PathBuf>,
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn days(now: i64, t: i64) -> Option<i64> {
    (t > 0).then(|| ((now - t).max(0)) / 86_400)
}

/// The snapshot, read once per call.
struct Snap {
    v: Value,
    home: Option<PathBuf>,
    now: i64,
}

impl Snap {
    fn scan(&self) -> Option<&Value> {
        self.v.get("scan").filter(|s| !s.is_null())
    }

    fn dev(&self) -> Option<&Value> {
        self.v.get("dev").filter(|s| !s.is_null())
    }

    fn arr<'a>(v: Option<&'a Value>, k: &str) -> &'a [Value] {
        v.and_then(|v| v.get(k)).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    fn project_json(&self, p: &Value) -> Value {
        json!({"root": p["root"], "git": p["git"], "worktree": p["worktree"], "uncommitted_changes": p["dirty"], "unpushed_commits": p["unpushed"], "idle_days": days(self.now, p["modified"].as_i64().unwrap_or(0)), "bytes": u(p, "bytes"), "bytes_human": human(u(p, "bytes"))})
    }

    /// Every build folder and cache in the report that the safety rules allow, by path.
    fn candidates(&self) -> HashMap<PathBuf, Candidate> {
        let mut out = HashMap::new();
        let mut add = |a: &Value, project: Option<Value>| {
            let Some(path) = str_of(a, "path").map(PathBuf::from) else { return };
            if crate::safety::protected(&path, self.home.as_deref()) {
                return;
            }
            out.insert(path, Candidate { kind: str_of(a, "kind").unwrap_or("").to_string(), bytes: u(a, "bytes"), modified: a["modified"].as_i64().unwrap_or(0), project });
        };
        for p in Snap::arr(self.dev(), "projects") {
            for a in p.get("build_folders").and_then(Value::as_array).into_iter().flatten() {
                add(a, Some(p.clone()));
            }
        }
        for c in Snap::arr(self.dev(), "caches") {
            add(c, None);
        }
        out
    }
}

fn limit(a: &Value, default: usize) -> usize {
    a.get("limit").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(default)
}

impl Server {
    pub fn new(snapshot: PathBuf, inbox: Option<PathBuf>) -> Server {
        Server { snapshot, inbox }
    }

    /// Parses `mcp --snapshot <file> [--propose] [--inbox <dir>] [--lang <code>]`.
    pub fn from_args(args: &[String]) -> Result<Server, String> {
        let (mut snapshot, mut inbox, mut propose) = (None, None, false);
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().cloned().ok_or_else(|| format!("{a} needs a value")); // i18n-ok: command line
            match a.as_str() {
                "--snapshot" => snapshot = Some(PathBuf::from(val()?)),
                "--inbox" => inbox = Some(PathBuf::from(val()?)),
                "--propose" => propose = true,
                "--lang" => {
                    val()?;
                }
                _ => return Err(format!("unknown argument {a}")), // i18n-ok: command line
            }
        }
        Ok(Server::new(snapshot.ok_or("--snapshot <file> is required")?, inbox.filter(|_| propose))) // i18n-ok: command line
    }

    pub fn tools(&self) -> Vec<Value> {
        let on = self.inbox.is_some();
        TOOLS.iter().filter(|t| on || !t.propose).map(|t| json!({"name": t.name, "description": t.description, "inputSchema": (t.schema)(), "annotations": {"readOnlyHint": !t.propose, "destructiveHint": false}})).collect()
    }

    fn load(&self) -> Result<Snap, String> {
        let s = std::fs::read_to_string(&self.snapshot).map_err(|e| format!("diskmap has not shared its data yet ({e}); ask the user to keep diskmap open"))?; // i18n-ok: for agents
        let v: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        let home = str_of(&v, "home").map(PathBuf::from);
        Ok(Snap { v, home, now: now_s() })
    }

    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?;
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        Some(match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
                let help = if self.inbox.is_some() { format!("{HELP}{PROPOSE_HELP}") } else { HELP.to_string() };
                ok(id, json!({"protocolVersion": version, "capabilities": {"tools": {"listChanged": false}}, "serverInfo": {"name": NAME, "version": env!("CARGO_PKG_VERSION")}, "instructions": help}))
            }
            "ping" => ok(id, json!({})),
            "tools/list" => ok(id, json!({"tools": self.tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().filter(|a| !a.is_null()).unwrap_or(json!({}));
                let Some(tool) = TOOLS.iter().find(|t| t.name == name && (self.inbox.is_some() || !t.propose)) else {
                    return Some(rpc_error(id, -32602, &format!("unknown tool {name:?}"))); // i18n-ok: protocol
                };
                let res = check_args(tool, &args).and_then(|_| self.load()).and_then(|snap| if tool.propose { self.call_propose(&snap, &args) } else { call_read(&snap, name, &args) });
                match res {
                    Ok(v) => ok(id, json!({"content": [{"type": "text", "text": v.to_string()}], "structuredContent": v, "isError": false})),
                    Err(e) => ok(id, json!({"content": [{"type": "text", "text": e}], "isError": true})),
                }
            }
            _ => rpc_error(id, -32601, &format!("method not found: {method}")), // i18n-ok: protocol
        })
    }

    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Array(batch)) => {
                    let r: Vec<Value> = batch.iter().filter_map(|m| self.handle(m)).collect();
                    (!r.is_empty()).then_some(Value::Array(r))
                }
                Ok(m) => self.handle(&m),
                Err(_) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}})), // i18n-ok: protocol
            };
            if let Some(r) = reply {
                writeln!(output, "{r}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Every path must be a cleanup candidate the safety rules allow; otherwise nothing is proposed.
    fn call_propose(&self, snap: &Snap, a: &Value) -> Result<Value, String> {
        let inbox = self.inbox.as_ref().ok_or("read-only session")?; // i18n-ok: for agents
        let paths: Vec<&str> = a["paths"].as_array().ok_or("paths must be a list")?.iter().map(|p| p.as_str().ok_or("paths must be strings")).collect::<Result<_, _>>()?; // i18n-ok: for agents
        if paths.is_empty() || paths.len() > MAX_PROPOSAL {
            return Err(format!("give 1 to {MAX_PROPOSAL} paths")); // i18n-ok: for agents
        }
        let cands = snap.candidates();
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut refused = Vec::new();
        for p in paths {
            let path = PathBuf::from(p);
            if !seen.insert(path.clone()) {
                continue;
            }
            match cands.get(&path) {
                Some(c) => items.push(Item { path, kind: c.kind.clone(), project: c.project.as_ref().and_then(|p| str_of(p, "root")).map(PathBuf::from), bytes: c.bytes }),
                None => refused.push(p.to_string()),
            }
        }
        if !refused.is_empty() {
            return Err(format!("not proposed: only paths from cleanup_candidates (regenerable build folders and caches) can go to the Trash; refused: {}. Tool settings and installs, ~/Library, cloud folders, personal files and project folders are never proposed.", refused.join(", "))); // i18n-ok: for agents
        }
        let why = str_of(a, "why").unwrap_or_default();
        let pr = write(inbox, items, why).map_err(|e| format!("diskmap is not listening for proposals ({e}); describe the folders to the user instead"))?; // i18n-ok: for agents
        Ok(json!({"pending": pr.id, "status": "awaiting_approval", "folders": pr.items.len(), "bytes": pr.bytes(), "bytes_human": human(pr.bytes()), "message": "Shown to the user as a card in diskmap; the folders go to the Trash only if they accept."})) // i18n-ok: for agents
    }
}

fn under(path: &str, root: Option<&Path>) -> bool {
    root.is_none_or(|r| Path::new(path).starts_with(r) && Path::new(path) != r)
}

fn call_read(snap: &Snap, name: &str, a: &Value) -> Result<Value, String> {
    let cands = snap.candidates();
    let scan = snap.scan();
    let no_scan = "no folder scanned yet: ask the user to scan one in diskmap (the start screen or Choose folder)"; // i18n-ok: for agents
    let kinds = |s: &Value| {
        let total = u(s, "bytes").max(1);
        Snap::arr(Some(s), "kinds").iter().map(|k| json!({"kind": k["kind"], "bytes": u(k, "bytes"), "bytes_human": human(u(k, "bytes")), "files": u(k, "files"), "share": (u(k, "bytes") as f64 / total as f64 * 1000.).round() / 1000.})).collect::<Vec<_>>()
    };
    let dev_totals = || {
        let dev = snap.dev();
        let builds: u64 = Snap::arr(dev, "projects").iter().map(|p| u(p, "bytes")).sum();
        let caches: u64 = Snap::arr(dev, "caches").iter().map(|c| u(c, "bytes")).sum();
        json!({"searching": snap.v["dev_searching"], "available": dev.is_some(), "projects": Snap::arr(dev, "projects").len(), "build_folders_bytes": builds, "build_folders_human": human(builds), "caches_bytes": caches, "caches_human": human(caches), "safe_to_remove_bytes": cands.values().map(|c| c.bytes).sum::<u64>()})
    };
    Ok(match name {
        "overview" => match scan {
            Some(s) => {
                let top: Vec<Value> = Snap::arr(Some(s), "folders").iter().filter(|f| u(f, "depth") == 1).take(15).map(|f| folder(f, &cands)).collect();
                json!({"scanned": s["root"], "bytes": u(s, "bytes"), "bytes_human": human(u(s, "bytes")), "files": u(s, "files"), "top_folders": top, "by_type": kinds(s), "dev_projects": dev_totals()})
            }
            None => json!({"scanned": null, "note": no_scan, "dev_projects": dev_totals()}),
        },
        "biggest" => {
            let s = scan.ok_or(no_scan)?;
            let root = str_of(a, "under").map(PathBuf::from);
            let n = limit(a, 20);
            if str_of(a, "what") == Some("files") {
                json!({"files": Snap::arr(Some(s), "largest_files").iter().filter(|f| under(str_of(f, "path").unwrap_or(""), root.as_deref())).take(n).map(|f| json!({"path": f["path"], "bytes": u(f, "bytes"), "bytes_human": human(u(f, "bytes")), "kind": f["kind"], "can_propose": false})).collect::<Vec<_>>(), "note": "personal files are never proposed for the Trash; describe them and let the user decide"}) // i18n-ok: for agents
            } else {
                json!({"folders": Snap::arr(Some(s), "folders").iter().filter(|f| under(str_of(f, "path").unwrap_or(""), root.as_deref())).take(n).map(|f| folder(f, &cands)).collect::<Vec<_>>()})
            }
        }
        "space_by_type" => {
            let s = scan.ok_or(no_scan)?;
            json!({"scanned": s["root"], "bytes": u(s, "bytes"), "by_type": kinds(s)})
        }
        "dev_projects" => {
            let dev = snap.dev().ok_or("the Developer projects search hasn't finished yet; try again in a moment")?; // i18n-ok: for agents
            let min = a.get("min_idle_days").and_then(Value::as_i64);
            let mut ps: Vec<&Value> = Snap::arr(Some(dev), "projects").iter().filter(|p| min.is_none_or(|m| days(snap.now, p["modified"].as_i64().unwrap_or(0)).is_none_or(|d| d >= m))).collect();
            if str_of(a, "sort") == Some("idle") {
                ps.sort_by_key(|p| p["modified"].as_i64().unwrap_or(0));
            }
            json!({"projects": ps.iter().take(limit(a, 30)).map(|p| {
                let mut j = snap.project_json(p);
                j["build_folders"] = json!(p["build_folders"].as_array().into_iter().flatten().map(|b| json!({"path": b["path"], "kind": b["kind"], "bytes": u(b, "bytes"), "bytes_human": human(u(b, "bytes")), "last_build_days": days(snap.now, b["modified"].as_i64().unwrap_or(0)), "can_propose": cands.contains_key(Path::new(str_of(b, "path").unwrap_or("")))})).collect::<Vec<_>>());
                j
            }).collect::<Vec<_>>(), "caches": Snap::arr(Some(dev), "caches").iter().map(|c| json!({"path": c["path"], "kind": c["kind"], "bytes": u(c, "bytes"), "bytes_human": human(u(c, "bytes"))})).collect::<Vec<_>>()})
        }
        "cleanup_candidates" => {
            if snap.dev().is_none() {
                return Err("the Developer projects search hasn't finished yet; try again in a moment".into()); // i18n-ok: for agents
            }
            let min = a.get("min_idle_days").and_then(Value::as_i64);
            let mut list: Vec<(&PathBuf, &Candidate)> = cands.iter().filter(|(_, c)| min.is_none_or(|m| days(snap.now, c.project.as_ref().and_then(|p| p["modified"].as_i64()).unwrap_or(c.modified)).is_none_or(|d| d >= m))).collect();
            list.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then(a.0.cmp(b.0)));
            let total: u64 = list.iter().map(|(_, c)| c.bytes).sum();
            json!({"total_bytes": total, "total_human": human(total), "candidates": list.iter().take(limit(a, 50)).map(|(p, c)| json!({"path": p, "kind": c.kind, "category": category(&c.kind), "regenerated": regenerated_by(&c.kind), "bytes": c.bytes, "bytes_human": human(c.bytes), "last_build_days": days(snap.now, c.modified), "project": c.project.as_ref().map(|p| snap.project_json(p))})).collect::<Vec<_>>(), "note": "nothing is selected for the user; propose only what they ask for"}) // i18n-ok: for agents
        }
        _ => return Err(format!("unknown tool {name:?}")), // i18n-ok: for agents
    })
}

fn folder(f: &Value, cands: &HashMap<PathBuf, Candidate>) -> Value {
    let path = str_of(f, "path").unwrap_or("");
    json!({"path": path, "bytes": u(f, "bytes"), "bytes_human": human(u(f, "bytes")), "files": u(f, "files"), "app_or_bundle": f["bundle"], "can_propose": cands.contains_key(Path::new(path))})
}

/// Arguments the tool doesn't take are refused with the list it does take.
fn check_args(t: &Tool, a: &Value) -> Result<(), String> {
    let Some(obj) = a.as_object() else { return Err("arguments must be an object".into()) }; // i18n-ok: for agents
    let schema = (t.schema)();
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    for k in obj.keys() {
        if !props.contains_key(k) {
            return Err(format!("{} does not take {k:?}; it takes: {}", t.name, props.keys().cloned().collect::<Vec<_>>().join(", "))); // i18n-ok: for agents
        }
    }
    for r in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !obj.contains_key(r) {
            return Err(format!("{} needs {r:?}", t.name)); // i18n-ok: for agents
        }
    }
    Ok(())
}
