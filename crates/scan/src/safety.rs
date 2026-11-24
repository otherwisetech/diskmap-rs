//! Paths diskmap must never offer to delete, whatever finds them (the Developer projects
//! search, its cleanup, or any assistant / MCP tool built on this crate). Tool configs and
//! installs, hidden folders in the home folder, the system, app bundles, cloud-synced folders
//! and whole user folders. The only exceptions are the exact regenerable caches in
//! [`HOME_CACHE_ALLOW`]. Anything that proposes a path for deletion must check [`protected`].
use std::path::{Component, Path};

/// Hidden or system folders under the home folder that are regenerable caches and may be offered
/// (`/`-separated, relative to the home folder). Everything else hidden in home is protected.
pub const HOME_CACHE_ALLOW: &[&str] = &[".npm/_cacache", ".pnpm-store", ".gradle/caches", "Library/pnpm/store", "Library/Developer/Xcode/DerivedData", "AppData/Local/pnpm/store", "AppData/Local/npm-cache/_cacache"];

/// Folder names protected wherever they are (a backup of another home, a mounted disk…): tool
/// configs, installs, credentials and app data.
const NAMES: &[&str] = &[
    ".config", ".local", ".opencode", ".codex", ".pi", ".cursor", ".npm", ".nvm", ".rustup", ".cargo", ".bun", ".deno", ".serverless", ".docker", ".kube", ".aws", ".gcloud", ".azure", ".ssh", ".gnupg", ".kiri", ".ollama", ".otherwise", ".git", ".Trash", ".Trashes", "Keychains", "CloudStorage",
    "Mobile Documents",
];

/// Name prefixes protected wherever they are.
const PREFIXES: &[&str] = &[".vscode", ".pi-", ".pi_"];

/// Bundles and libraries: never a cleanup target, nor anything inside them.
const BUNDLE_SUFFIXES: &[&str] = &[".app", ".appex", ".framework", ".bundle", ".kext", ".plugin", ".photoslibrary", ".musiclibrary", ".keychain", ".keychain-db"];

/// Cloud-synced folders kept directly in the home folder (prefixes: `OneDrive - Acme`).
const CLOUD_IN_HOME: &[&str] = &["OneDrive", "Dropbox", "Google Drive", "iCloud Drive", "Box", "Box Sync", "pCloud Drive", "Nextcloud", "MEGA", "Creative Cloud Files"];

/// Folders under the home folder protected with everything in them.
const HOME_PROTECTED: &[&str] = &["Library", "AppData", "Applications", "Pictures", "Movies", "Music"];

/// System locations (first components of an absolute path) never offered.
#[cfg(unix)]
const SYSTEM: &[&str] = &["System", "Library", "Applications", "usr", "bin", "sbin", "opt", "private", "etc", "var", "dev", "cores", "nix", "snap", "boot", "proc", "sys", "lib", "lib64", "root"];
#[cfg(windows)]
const SYSTEM: &[&str] = &["Windows", "Program Files", "Program Files (x86)", "ProgramData"];
#[cfg(not(any(unix, windows)))]
const SYSTEM: &[&str] = &[];

fn names(p: &Path) -> Vec<&str> {
    p.components().filter_map(|c| if let Component::Normal(n) = c { n.to_str().or(Some("\u{FFFD}")) } else { None }).collect()
}

fn is_cloud_name(n: &str) -> bool {
    CLOUD_IN_HOME.iter().any(|c| n == *c || (n.starts_with(c) && n[c.len()..].starts_with([' ', '-', '_', '(', '.'])))
}

/// `true` when `path` (absolute, with no `..`) must never be offered for deletion. `home` is the
/// user's home folder (canonical), or `None` when unknown (then only the generic rules apply).
pub fn protected(path: &Path, home: Option<&Path>) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return true;
    }
    let all = names(path);
    if all.iter().any(|n| NAMES.contains(n) || PREFIXES.iter().any(|p| n.starts_with(p)) || BUNDLE_SUFFIXES.iter().any(|s| n.len() > s.len() && n.ends_with(s))) {
        // the exact regenerable caches under hidden home folders are the only way in
        return !home.is_some_and(|h| allowed_cache(path, h));
    }
    let by_place = match home {
        Some(h) if path == h => true,
        Some(h) if path.starts_with(h) => in_home(path, h),
        _ => outside_home(&all),
    };
    by_place || looks_like_project(path)
}

/// Files that make a folder a source project: never offered as a whole, nor as a build folder.
pub const PROJECT_MARKERS: &[&str] = &[".git", "package.json", "Cargo.toml", "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "pyproject.toml", "setup.py", "go.mod", "Gemfile", "composer.json", "Package.swift", "Podfile", "Makefile", "CMakeLists.txt", "deno.json"];

/// `dir` holds a project marker directly: it is (or looks like) source code, not build output.
pub fn looks_like_project(dir: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| std::fs::symlink_metadata(dir.join(m)).is_ok())
}

/// `true` when everything under `path` is protected too, so a search need not enter it. A
/// user folder (`~/Documents`) or the home folder is protected as a whole but searched inside.
pub fn protected_tree(path: &Path, home: Option<&Path>) -> bool {
    let probe = "\u{1}diskmap-probe";
    protected(&path.join(probe).join(probe), home)
}

fn allowed_cache(path: &Path, home: &Path) -> bool {
    HOME_CACHE_ALLOW.iter().any(|a| path.starts_with(home.join(a))) || standalone_target(path, home)
}

/// `~/.cache/<name with "target">` holding a `CACHEDIR.TAG`: a `CARGO_TARGET_DIR`.
fn standalone_target(path: &Path, home: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(home.join(".cache")) else { return false };
    let Some(first) = names(rel).first().map(|s| s.to_string()) else { return false };
    first.contains("target") && home.join(".cache").join(&first).join("CACHEDIR.TAG").is_file()
}

fn in_home(path: &Path, home: &Path) -> bool {
    if allowed_cache(path, home) {
        return false;
    }
    let rel = names(path.strip_prefix(home).unwrap_or(path));
    let Some(first) = rel.first() else { return true };
    // directly in home is a user folder, a project or a stray install, never a build folder
    first.starts_with('.') || HOME_PROTECTED.contains(first) || is_cloud_name(first) || rel.len() == 1
}

fn outside_home(all: &[&str]) -> bool {
    if all.is_empty() {
        return true; // a volume or drive root
    }
    if SYSTEM.contains(&all[0]) {
        return true;
    }
    // a mounted volume as a whole
    if matches!(all[0], "Volumes" | "mnt" | "media" | "run") && all.len() <= 2 + usize::from(all[0] == "media" || all[0] == "run") {
        return true;
    }
    // other users' home folders on this disk or a mounted one: same rules as ours
    if let Some(i) = all.iter().position(|n| *n == "Users" || *n == "home") {
        let rest = &all[i + 1..];
        if rest.len() <= 1 {
            return true;
        }
        let first = rest[1];
        if first.starts_with('.') || HOME_PROTECTED.contains(&first) || is_cloud_name(first) || rest.len() == 2 {
            return true;
        }
    }
    false
}
