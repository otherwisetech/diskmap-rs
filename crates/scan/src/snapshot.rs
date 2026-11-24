//! What the AI panel's MCP server reads: a pruned copy of the window's scan (the biggest folders
//! and files, space by type) and the Developer projects report with git state. The window writes
//! it into its private session folder; `diskmap mcp` reads only that file, never the disk.
use crate::dev::{Artifact, GitState, Report};
use crate::{Kind, NodeId, Tree};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const VERSION: u64 = 1;
/// Most folders / files kept from a scan.
pub const MAX_FOLDERS: usize = 400;
pub const MAX_FILES: usize = 200;

/// The scan, pruned: folders of at least 1/2000 of the root (largest [`MAX_FOLDERS`]), the
/// largest files and the space by type.
pub fn scan_json(tree: &Tree) -> Value {
    let root = tree.node(Tree::ROOT);
    let floor = root.size / 2000;
    let mut folders: Vec<(NodeId, usize)> = Vec::new();
    let mut kinds: HashMap<Kind, (u64, u64)> = HashMap::new();
    let mut stack: Vec<(NodeId, usize)> = vec![(Tree::ROOT, 0)];
    while let Some((id, depth)) = stack.pop() {
        let n = tree.node(id);
        if n.is_dir() && n.kind != Kind::Package {
            if id != Tree::ROOT && n.size >= floor {
                folders.push((id, depth));
            }
            stack.extend(n.children.iter().map(|&c| (c, depth + 1)));
        } else {
            let e = kinds.entry(n.kind).or_default();
            e.0 += n.size;
            e.1 += n.files.max(1);
            if n.kind == Kind::Package && id != Tree::ROOT && n.size >= floor {
                folders.push((id, depth));
            }
        }
    }
    folders.sort_by(|a, b| tree.node(b.0).size.cmp(&tree.node(a.0).size).then(a.0.cmp(&b.0)));
    folders.truncate(MAX_FOLDERS);
    let mut kinds: Vec<(Kind, (u64, u64))> = kinds.into_iter().collect();
    kinds.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.id().cmp(b.0.id())));
    json!({
        "root": tree.root_path(),
        "bytes": root.size,
        "files": root.files,
        "folders": folders.iter().map(|&(id, depth)| { let n = tree.node(id); json!({"path": tree.path(id), "bytes": n.size, "files": n.files, "depth": depth, "bundle": n.kind == Kind::Package}) }).collect::<Vec<_>>(),
        "largest_files": tree.largest_files(Tree::ROOT, MAX_FILES).into_iter().map(|id| { let n = tree.node(id); json!({"path": tree.path(id), "bytes": n.size, "kind": n.kind.id()}) }).collect::<Vec<_>>(),
        "kinds": kinds.iter().map(|(k, (b, f))| json!({"kind": k.id(), "bytes": b, "files": f})).collect::<Vec<_>>(),
    })
}

fn artifact(a: &Artifact) -> Value {
    json!({"path": a.path, "kind": a.kind.id(), "bytes": a.bytes, "files": a.files, "modified": a.modified})
}

/// The Developer projects report, with what git says about each repository.
pub fn dev_json(r: &Report, git: &HashMap<PathBuf, GitState>) -> Value {
    json!({
        "projects": r.projects.iter().map(|p| {
            let g = git.get(&p.root).copied().unwrap_or_default();
            json!({"root": p.root, "git": p.git, "worktree": p.worktree, "dirty": g.dirty, "unpushed": g.unpushed, "modified": p.modified, "bytes": p.bytes, "build_folders": p.artifacts.iter().map(artifact).collect::<Vec<_>>()})
        }).collect::<Vec<_>>(),
        "caches": r.caches.iter().map(artifact).collect::<Vec<_>>(),
    })
}

/// The whole snapshot. `dev` is `None` while the search hasn't finished (`searching`).
pub fn snapshot_json(home: &Path, scan: Option<Value>, dev: Option<Value>, searching: bool, now: i64) -> String {
    json!({"version": VERSION, "home": home, "written_at": now, "scan": scan, "dev": dev, "dev_searching": searching}).to_string()
}
