//! Parallel directory walk. Never follows symlinks; hard links are counted once.
use crate::{Kind, Node, NodeId, Tree};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

#[derive(Debug, Clone)]
pub struct Options {
    /// Don't descend into directories on another device than the scan root.
    pub one_filesystem: bool,
    /// Absolute paths never entered (pseudo filesystems, other volumes' mount points).
    pub skip: Vec<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        // macOS: /System/Volumes/Data is reachable again through firmlinks (/Users, /Applications…),
        // so scanning "/" would count it twice. /Volumes holds other disks.
        #[cfg(target_os = "macos")]
        let skip = ["/System/Volumes", "/Volumes", "/dev", "/.vol", "/.nofollow", "/.resolve"];
        #[cfg(all(unix, not(target_os = "macos")))]
        let skip = ["/proc", "/sys", "/dev", "/run", "/snap", "/tmp/.mount"];
        #[cfg(not(unix))]
        let skip: [&str; 0] = [];
        Options { one_filesystem: false, skip: skip.iter().map(PathBuf::from).collect() }
    }
}

/// Live counters a UI can poll while [`scan`] runs on another thread; `cancel` stops it early.
#[derive(Debug, Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
}

struct Entry {
    name: Box<str>,
    size: u64,
    files: u64,
    kind: Kind,
    unreadable: bool,
    children: Vec<Entry>,
}

struct Ctx<'a> {
    opts: &'a Options,
    progress: &'a Progress,
    root_dev: u64,
    #[cfg_attr(not(unix), allow(dead_code))] // hard links are only detected on unix
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
}

/// Scan `root` (a directory) into a [`Tree`]. Unreadable subdirectories are kept and flagged;
/// only a root that cannot be read is an error. A cancelled scan returns `ErrorKind::Interrupted`.
pub fn scan(root: &Path, opts: &Options, progress: &Progress) -> io::Result<Tree> {
    let root = fs::canonicalize(root)?;
    let meta = fs::metadata(&root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotADirectory, root.display().to_string()));
    }
    fs::read_dir(&root)?;
    let ctx = Ctx { opts, progress, root_dev: dev(&meta), seen_inodes: Mutex::new(HashSet::new()) };
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
    let kind = Kind::for_dir(&name);
    let entry = walk_dir(&ctx, &root, name.into(), allocated(&meta), kind);
    if progress.cancel.load(Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let mut nodes = Vec::new();
    flatten(entry, None, &mut nodes);
    Ok(Tree { nodes, root_path: root })
}

fn walk_dir(ctx: &Ctx, path: &Path, name: Box<str>, own: u64, kind: Kind) -> Entry {
    ctx.progress.dirs.fetch_add(1, Relaxed);
    ctx.progress.bytes.fetch_add(own, Relaxed);
    let mut e = Entry { name, size: own, files: 0, kind, unreadable: false, children: Vec::new() };
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => {
            ctx.progress.errors.fetch_add(1, Relaxed);
            e.unreadable = true;
            return e;
        }
    };
    let mut subdirs: Vec<(PathBuf, Box<str>, u64, Kind)> = Vec::new();
    for item in rd {
        if ctx.progress.cancel.load(Relaxed) {
            return e;
        }
        let Ok(item) = item else {
            e.unreadable = true;
            continue;
        };
        // DirEntry::metadata does not traverse symlinks
        let Ok(meta) = item.metadata() else {
            ctx.progress.errors.fetch_add(1, Relaxed);
            continue;
        };
        let name: Box<str> = item.file_name().to_string_lossy().into();
        if meta.is_dir() {
            let p = item.path();
            if ctx.opts.skip.contains(&p) || (ctx.opts.one_filesystem && dev(&meta) != ctx.root_dev) {
                continue;
            }
            let kind = Kind::for_dir(&name);
            subdirs.push((p, name, allocated(&meta), kind));
        } else {
            let size = if is_repeat_link(ctx, &meta) { 0 } else { allocated(&meta) };
            ctx.progress.files.fetch_add(1, Relaxed);
            ctx.progress.bytes.fetch_add(size, Relaxed);
            let kind = Kind::for_file(&name);
            e.children.push(Entry { name, size, files: 1, kind, unreadable: false, children: Vec::new() });
        }
    }
    let dirs: Vec<Entry> = subdirs.into_par_iter().map(|(p, name, own, kind)| walk_dir(ctx, &p, name, own, kind)).collect();
    e.children.extend(dirs);
    for c in &e.children {
        e.size += c.size;
        e.files += c.files;
    }
    e
}

fn flatten(e: Entry, parent: Option<NodeId>, nodes: &mut Vec<Node>) -> NodeId {
    let id = nodes.len() as NodeId;
    let Entry { name, size, files, kind, unreadable, mut children } = e;
    nodes.push(Node { name, parent, children: Vec::new(), size, files, kind, unreadable, removed: false });
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    let ids: Vec<NodeId> = children.into_iter().map(|c| flatten(c, Some(id), nodes)).collect();
    nodes[id as usize].children = ids;
    id
}

#[cfg(unix)]
fn allocated(m: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(m: &fs::Metadata) -> u64 {
    m.len()
}

#[cfg(unix)]
fn dev(m: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.dev()
}

#[cfg(not(unix))]
fn dev(_: &fs::Metadata) -> u64 {
    0
}

#[cfg(unix)]
fn is_repeat_link(ctx: &Ctx, m: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.nlink() > 1 && !ctx.seen_inodes.lock().unwrap().insert((m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn is_repeat_link(_: &Ctx, _: &fs::Metadata) -> bool {
    false
}
