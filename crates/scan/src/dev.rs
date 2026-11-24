//! Developer projects: regenerable build folders (`target/`, `node_modules/`, `.next/`, Gradle's
//! `.gradle/` and `build/`, `DerivedData`) under some roots, grouped by project (the git root
//! above them, else the folder holding them) with sizes and the project's last change; plus the
//! package caches and standalone cargo target folders in the home folder. Never follows
//! symlinks, counts hard links once. Read-only except [`delete`], which re-checks everything.
//! Nothing [`crate::safety::protected`] is ever searched, reported or deleted: tool configs and
//! installs that carry a `node_modules` (`~/.config/opencode`, `~/.vscode-server`…) are not projects.
use crate::Progress;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::Ordering::Relaxed;

/// Directory depth below a root that is searched for projects.
const MAX_DEPTH: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtifactKind {
    /// `target/` of cargo (tagged `CACHEDIR.TAG` or next to `Cargo.toml`) or maven (`pom.xml`).
    Target,
    NodeModules,
    /// `.next/` next to a `package.json`.
    Next,
    /// `.gradle/` next to a Gradle build file.
    Gradle,
    /// `build/` next to a Gradle build file.
    GradleBuild,
    /// Xcode's `DerivedData` (per project, or `~/Library/Developer/Xcode/DerivedData`).
    DerivedData,
    /// A cargo target folder of its own, like `~/.cache/<name>target<…>` (`CARGO_TARGET_DIR`).
    StandaloneTarget,
    NpmCache,
    PnpmStore,
    GradleCache,
}

impl ArtifactKind {
    /// Stable id, for i18n keys (`app.dev.kind.<id>`).
    pub fn id(self) -> &'static str {
        match self {
            ArtifactKind::Target => "target",
            ArtifactKind::NodeModules => "node_modules",
            ArtifactKind::Next => "next",
            ArtifactKind::Gradle => "gradle",
            ArtifactKind::GradleBuild => "gradle_build",
            ArtifactKind::DerivedData => "derived_data",
            ArtifactKind::StandaloneTarget => "standalone_target",
            ArtifactKind::NpmCache => "npm_cache",
            ArtifactKind::PnpmStore => "pnpm_store",
            ArtifactKind::GradleCache => "gradle_cache",
        }
    }

    /// Downloaded packages shared by every project: regenerable, but re-downloading costs.
    pub fn is_download_cache(self) -> bool {
        matches!(self, ArtifactKind::NpmCache | ArtifactKind::PnpmStore | ArtifactKind::GradleCache)
    }

    pub const ALL: [ArtifactKind; 10] = [
        ArtifactKind::Target,
        ArtifactKind::NodeModules,
        ArtifactKind::Next,
        ArtifactKind::Gradle,
        ArtifactKind::GradleBuild,
        ArtifactKind::DerivedData,
        ArtifactKind::StandaloneTarget,
        ArtifactKind::NpmCache,
        ArtifactKind::PnpmStore,
        ArtifactKind::GradleCache,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub bytes: u64,
    pub files: u64,
    /// Newest modification time inside (seconds since the epoch): its last build / install.
    pub modified: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    /// `root` holds a `.git` (a folder, or a file for a linked worktree).
    pub git: bool,
    /// A linked worktree (`.git` is a file).
    pub worktree: bool,
    /// Newest change outside its build folders (seconds since the epoch; 0 if none found).
    pub modified: i64,
    /// Sorted by size, largest first.
    pub artifacts: Vec<Artifact>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Largest first.
    pub projects: Vec<Project>,
    /// Package caches and standalone target folders in the home folder, largest first.
    pub caches: Vec<Artifact>,
}

impl Report {
    pub fn total(&self) -> u64 {
        self.projects.iter().map(|p| p.bytes).sum::<u64>() + self.caches.iter().map(|c| c.bytes).sum::<u64>()
    }

    /// Build output only (no download caches): what deleting costs a rebuild, not a download.
    pub fn build_bytes(&self) -> u64 {
        self.projects.iter().map(|p| p.bytes).sum::<u64>() + self.caches.iter().filter(|c| !c.kind.is_download_cache()).map(|c| c.bytes).sum::<u64>()
    }
}

/// `fs::canonicalize` without Windows' `\\?\` prefix (git and the shell don't take it).
pub fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    let c = fs::canonicalize(p)?;
    #[cfg(windows)]
    if let Some(rest) = c.to_str().and_then(|s| s.strip_prefix(r"\\?\")).filter(|r| !r.starts_with("UNC")) {
        return Ok(PathBuf::from(rest));
    }
    Ok(c)
}

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| dir.join(n).is_file())
}

const GRADLE_FILES: &[&str] = &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"];

/// What kind of build folder `dir` is, judged by its name and the files next to it.
pub fn classify(dir: &Path) -> Option<ArtifactKind> {
    let name = dir.file_name()?.to_str()?;
    let parent = dir.parent()?;
    match name {
        // only a project's own install (a `package.json` next to it); global installs aren't
        "node_modules" if parent.join("package.json").is_file() => Some(ArtifactKind::NodeModules),
        "target" if dir.join("CACHEDIR.TAG").is_file() || has_any(parent, &["Cargo.toml", "pom.xml"]) => Some(ArtifactKind::Target),
        ".next" if parent.join("package.json").is_file() => Some(ArtifactKind::Next),
        ".gradle" if has_any(parent, GRADLE_FILES) => Some(ArtifactKind::Gradle),
        "build" if has_any(parent, &["build.gradle", "build.gradle.kts"]) => Some(ArtifactKind::GradleBuild),
        "DerivedData" if parent.file_name().is_some_and(|n| n != "Xcode") => Some(ArtifactKind::DerivedData),
        _ => None,
    }
}

pub use crate::safety::{PROJECT_MARKERS, looks_like_project};

/// The caches looked for in the home folder. Only exact, regenerable cache folders: never a
/// tool's install or config (`~/.cargo` and `~/.local` hold binaries and settings too).
pub fn home_caches(home: &Path) -> Vec<(PathBuf, ArtifactKind)> {
    let mut v: Vec<(PathBuf, ArtifactKind)> = [
        (".npm/_cacache", ArtifactKind::NpmCache),
        ("Library/pnpm/store", ArtifactKind::PnpmStore),
        (".pnpm-store", ArtifactKind::PnpmStore),
        ("AppData/Local/pnpm/store", ArtifactKind::PnpmStore),
        ("AppData/Local/npm-cache/_cacache", ArtifactKind::NpmCache),
        (".gradle/caches", ArtifactKind::GradleCache),
        ("Library/Developer/Xcode/DerivedData", ArtifactKind::DerivedData),
    ]
    .into_iter()
    .map(|(p, k)| (home.join(p), k))
    .collect();
    // `CARGO_TARGET_DIR`s people keep in ~/.cache (`~/.cache/myapp-target`)
    if let Ok(rd) = fs::read_dir(home.join(".cache")) {
        let mut found: Vec<PathBuf> = rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name().to_string_lossy().contains("target")).map(|e| e.path()).filter(|p| p.join("CACHEDIR.TAG").is_file()).collect();
        found.sort();
        v.extend(found.into_iter().map(|p| (p, ArtifactKind::StandaloneTarget)));
    }
    v.retain(|(p, _)| !crate::safety::protected(p, Some(home)));
    v
}

/// Folder names never searched for projects (relative to the home folder when `home_only`).
fn skipped(name: &str, at_home: bool) -> bool {
    const ANYWHERE: &[&str] = &[".git", ".Trash", ".Trashes", "$RECYCLE.BIN", ".hg", ".svn"];
    const IN_HOME: &[&str] = &["Library", "AppData", ".cache", ".cargo", ".rustup", ".npm", ".pnpm-store", ".gradle", ".local", ".m2", "Applications", "Movies", "Music", "Pictures", ".docker", ".orbstack", "OrbStack"];
    ANYWHERE.contains(&name) || (at_home && IN_HOME.contains(&name)) || name.ends_with(".app") || name.ends_with(".photoslibrary") || name.ends_with(".musiclibrary")
}

struct Ctx<'a> {
    progress: &'a Progress,
    home: Option<PathBuf>,
    skip: Vec<PathBuf>,
    seen: Mutex<HashSet<(u64, u64)>>,
}

/// Build folders found under a folder, owned by the git root above them once one is found.
#[derive(Default)]
struct Found {
    projects: Vec<Project>,
    /// (folder holding it, that folder's newest change, artifact) not yet inside a git root.
    loose: Vec<(PathBuf, i64, Artifact)>,
    /// Newest change outside build folders and nested git roots.
    newest: i64,
}

fn mtime(md: &fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(unix)]
fn allocated(md: &fs::Metadata, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    use std::os::unix::fs::MetadataExt;
    if md.nlink() > 1 && !md.is_dir() && !seen.lock().unwrap().insert((md.dev(), md.ino())) {
        return 0;
    }
    md.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(md: &fs::Metadata, _: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    md.len()
}

/// Size, file count and newest change of everything under `dir`.
fn measure(ctx: &Ctx, dir: &Path, kind: ArtifactKind) -> Artifact {
    let mut a = Artifact { path: dir.to_path_buf(), kind, bytes: 0, files: 0, modified: 0 };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if ctx.progress.cancel.load(Relaxed) {
            break;
        }
        ctx.progress.dirs.fetch_add(1, Relaxed);
        let Ok(rd) = fs::read_dir(&d) else {
            ctx.progress.errors.fetch_add(1, Relaxed);
            continue;
        };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            let size = allocated(&md, &ctx.seen);
            a.bytes += size;
            a.modified = a.modified.max(mtime(&md));
            ctx.progress.bytes.fetch_add(size, Relaxed);
            if md.is_dir() {
                stack.push(e.path());
            } else {
                a.files += 1;
                ctx.progress.files.fetch_add(1, Relaxed);
            }
        }
    }
    a
}

fn visit(ctx: &Ctx, dir: &Path, depth: usize) -> Found {
    let mut out = Found::default();
    if ctx.progress.cancel.load(Relaxed) {
        return out;
    }
    ctx.progress.dirs.fetch_add(1, Relaxed);
    let Ok(rd) = fs::read_dir(dir) else {
        ctx.progress.errors.fetch_add(1, Relaxed);
        return out;
    };
    let at_home = ctx.home.as_deref() == Some(dir);
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut here: Vec<(PathBuf, ArtifactKind)> = Vec::new();
    let mut git: Option<bool> = None;
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            git = Some(md.is_file());
            continue;
        }
        if !md.is_dir() {
            ctx.progress.files.fetch_add(1, Relaxed);
            out.newest = out.newest.max(mtime(&md));
            continue;
        }
        let p = e.path();
        if skipped(&name, at_home) || ctx.skip.contains(&p) || crate::safety::protected_tree(&p, ctx.home.as_deref()) {
            continue;
        }
        match classify(&p) {
            Some(k) if !looks_like_project(&p) && !crate::safety::protected(&p, ctx.home.as_deref()) => here.push((p, k)),
            Some(_) => {}
            None if depth < MAX_DEPTH => subdirs.push(p),
            None => {}
        }
    }
    let arts: Vec<Artifact> = here.par_iter().map(|(p, k)| measure(ctx, p, *k)).collect();
    let kids: Vec<Found> = subdirs.par_iter().map(|p| visit(ctx, p, depth + 1)).collect();
    for k in kids {
        out.projects.extend(k.projects);
        out.loose.extend(k.loose);
        out.newest = out.newest.max(k.newest);
    }
    let newest = out.newest;
    out.loose.extend(arts.into_iter().map(|a| (dir.to_path_buf(), newest, a)));
    if let Some(worktree) = git {
        let artifacts: Vec<Artifact> = std::mem::take(&mut out.loose).into_iter().map(|(_, _, a)| a).collect();
        if !artifacts.is_empty() {
            out.projects.push(project(dir.to_path_buf(), true, worktree, newest, artifacts));
        }
        out.newest = 0;
    }
    out
}

fn project(root: PathBuf, git: bool, worktree: bool, modified: i64, mut artifacts: Vec<Artifact>) -> Project {
    artifacts.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    let bytes = artifacts.iter().map(|a| a.bytes).sum();
    Project { root, git, worktree, modified, artifacts, bytes }
}

/// Search `roots` for projects with build folders, and `home` for package caches. Roots inside
/// another root are searched once. Cancelled through `progress.cancel` (the result is partial).
pub fn find(roots: &[PathBuf], home: Option<&Path>, progress: &Progress) -> Report {
    let mut roots: Vec<PathBuf> = roots.iter().filter_map(|r| canonical(r).ok()).filter(|r| r.is_dir()).collect();
    roots.sort();
    roots.dedup();
    let roots: Vec<PathBuf> = roots.iter().filter(|r| !roots.iter().any(|o| o != *r && r.starts_with(o))).cloned().collect();
    let home = home.and_then(|h| canonical(h).ok());
    // a protected root (`~/.config`, iCloud Drive…) is not searched; home and ~/Documents are,
    // with their protected folders left out one by one
    let roots: Vec<PathBuf> = roots.into_iter().filter(|r| !crate::safety::protected_tree(r, home.as_deref())).collect();
    let ctx = Ctx { progress, home: home.clone(), skip: crate::Options::default().skip, seen: Mutex::new(HashSet::new()) };
    let caches: Vec<(PathBuf, ArtifactKind)> = home.as_deref().map(home_caches).unwrap_or_default().into_iter().filter(|(p, _)| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())).collect();
    let (found, caches) = rayon::join(|| roots.par_iter().map(|r| visit(&ctx, r, 0)).collect::<Vec<_>>(), || caches.par_iter().map(|(p, k)| measure(&ctx, p, *k)).collect::<Vec<_>>());
    let mut projects: Vec<Project> = Vec::new();
    for f in found {
        projects.extend(f.projects);
        // build folders outside any git repository belong to the folder holding them
        let mut loose = f.loose;
        loose.sort_by(|a, b| a.0.cmp(&b.0));
        let mut i = 0;
        while i < loose.len() {
            let owner = loose[i].0.clone();
            let newest = loose[i].1;
            let mut arts = Vec::new();
            while i < loose.len() && loose[i].0 == owner {
                arts.push(loose[i].2.clone());
                i += 1;
            }
            projects.push(project(owner, false, false, newest, arts));
        }
    }
    projects.retain(|p| p.bytes > 0);
    projects.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.root.cmp(&b.root)));
    let mut caches: Vec<Artifact> = caches.into_iter().filter(|c| c.bytes > 0).collect();
    caches.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    Report { projects, caches }
}

/// What git says about a project, for the assistant: `None` when git couldn't tell (or, for
/// `unpushed`, the branch has no upstream).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GitState {
    pub dirty: Option<bool>,
    pub unpushed: Option<bool>,
}

/// Uncommitted changes (untracked files included) and commits not on the upstream.
pub fn git_state(root: &Path) -> GitState {
    let run = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(root).arg("--no-optional-locks").args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| o.stdout);
    let dirty = run(&["status", "--porcelain"]).map(|o| !o.is_empty());
    let unpushed = run(&["rev-list", "--count", "@{upstream}..HEAD"]).and_then(|o| String::from_utf8_lossy(&o).trim().parse::<u64>().ok()).map(|n| n > 0);
    GitState { dirty, unpushed }
}

/// Why [`delete_with`] left a folder alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not (anymore) a build folder of that kind, a symlink, or outside its project.
    NotArtifact,
    /// A path diskmap never deletes (tool config or install, hidden home folder, cloud…).
    Protected,
    /// git tracks files in it, or sees files there that aren't ignored.
    Changed,
    Failed(String),
}

/// git's view of `path` inside the repository at `root`: `true` when nothing under it is
/// tracked and nothing there is untracked-but-not-ignored. `None` when git can't tell.
pub fn git_ignores_all(root: &Path, path: &Path) -> Option<bool> {
    let rel = path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
    let run = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(root).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| o.stdout);
    let tracked = run(&["ls-files", "-z", "--", &rel])?;
    let status = run(&["--no-optional-locks", "status", "--porcelain", "-z", "--untracked-files=all", "--", &rel])?;
    Some(tracked.is_empty() && status.is_empty())
}

/// Check one build folder or cache right before it goes, then hand it to `remove` (the app
/// moves it to the Trash; tests remove temp folders). Refuses anything protected
/// ([`crate::safety::protected`]) or holding a project marker, and unless `a.path` is still a real
/// directory (not a symlink, nothing linked on the way) of `a.kind` — a project build folder
/// inside `project` (and, in a git repository, entirely ignored by git), or exactly one of the
/// home caches.
pub fn delete_with(a: &Artifact, project: Option<&Project>, home: Option<&Path>, remove: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), Refusal> {
    let md = fs::symlink_metadata(&a.path).map_err(|e| Refusal::Failed(e.to_string()))?;
    if md.file_type().is_symlink() || !md.is_dir() {
        return Err(Refusal::NotArtifact);
    }
    let real = canonical(&a.path).map_err(|e| Refusal::Failed(e.to_string()))?;
    if real != a.path {
        return Err(Refusal::NotArtifact);
    }
    let home = home.and_then(|h| canonical(h).ok());
    if crate::safety::protected(&real, home.as_deref()) {
        return Err(Refusal::Protected);
    }
    match project {
        Some(p) => {
            if !real.starts_with(&p.root) || real == p.root || classify(&real) != Some(a.kind) {
                return Err(Refusal::NotArtifact);
            }
            if p.git && git_ignores_all(&p.root, &real) != Some(true) {
                return Err(Refusal::Changed);
            }
        }
        None => {
            let home = home.ok_or(Refusal::NotArtifact)?;
            if !home_caches(&home).iter().any(|(p, k)| *p == real && *k == a.kind) {
                return Err(Refusal::NotArtifact);
            }
        }
    }
    remove(&real).map_err(Refusal::Failed)
}
