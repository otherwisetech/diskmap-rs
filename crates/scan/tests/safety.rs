//! What the Developer projects cleanup may never offer: a fake home in a temp dir full of tool
//! configs and installs (each carrying a `package.json` + `node_modules` and a cargo `target/`, so
//! a careless search would find them), cloud folders and git projects. Never the real home.
use scan::Progress;
use scan::dev::{self, Refusal};
use scan::safety::{protected, protected_tree};
use std::fs;
use std::path::{Path, PathBuf};

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

/// Something that looks like a node + cargo project with build folders at `dir`.
fn fake_project(dir: &Path) {
    write(&dir.join("package.json"), 10);
    write(&dir.join("node_modules/dep/index.js"), 20_000);
    write(&dir.join("Cargo.toml"), 10);
    write(&dir.join("target/CACHEDIR.TAG"), 10);
    write(&dir.join("target/debug/x"), 20_000);
}

/// Every excluded place, relative to the home folder.
const EXCLUDED: &[&str] = &[
    ".config/opencode",
    ".config/some-tool",
    ".local/share/app",
    ".local/state/x",
    ".opencode",
    ".claude",
    ".claude-work",
    ".claude/worktrees/x",
    ".codex",
    ".pi",
    ".pi-agent",
    ".pi/agent",
    ".cursor/extensions/e",
    ".vscode/extensions/e",
    ".vscode-server/cli/servers/Stable-abc/server",
    ".vscode-insiders",
    ".npm/_npx/abc",
    ".npm",
    ".nvm/versions/node/v20.1.0/lib",
    ".rustup/toolchains/stable",
    ".cargo/bin",
    ".cargo/registry/src/x",
    ".cargo/git/checkouts/x",
    ".bun/install/global",
    ".deno",
    ".serverless/releases/4.17.1/package",
    ".docker",
    ".kube",
    ".aws",
    ".gcloud",
    ".azure",
    ".ssh",
    ".gnupg",
    ".kiri",
    ".ollama/models",
    ".otherwise/diskmap",
    ".some-new-tool",
    ".cache/not-a-cargo-target",
    "Library/Application Support/Code/User",
    "Library/Keychains",
    "Library/Containers/com.x/Data",
    "Library/CloudStorage/OneDrive-Acme/proj",
    "Library/Mobile Documents/com~apple~CloudDocs/proj",
    "OneDrive - Acme/proj",
    "OneDrive/proj",
    "Dropbox/proj",
    "Google Drive/proj",
    "Applications/Tool.app/Contents/Resources/app",
    "code/Thing.app/Contents/Resources/app",
    "code/Lib.framework/Resources",
    "Pictures/proj",
    "Movies/proj",
    "Music/proj",
];

/// Folders offered only for what's inside them, never as a whole.
const WHOLE: &[&str] = &["", "Documents", "Desktop", "Downloads", "Pictures", "Public", "Applications", "Library", "Library/Caches", "Library/Developer/Xcode", "Vcity/aliexpress-scraper", "Downloads/27-shaders", "code/app"];

#[test]
fn excluded_paths_are_never_candidates() {
    let d = tempfile::tempdir().unwrap();
    let home = dev::canonical(d.path()).unwrap();
    for e in EXCLUDED {
        fake_project(&home.join(e));
    }
    // a cargo target kept in ~/.cache, with its tag: the one hidden-home build folder allowed
    write(&home.join(".cache/app-target/CACHEDIR.TAG"), 10);
    write(&home.join(".cache/app-target/debug/x"), 30_000);
    write(&home.join(".cache/not-a-cargo-target/x"), 30_000);
    write(&home.join(".npm/_cacache/index/x"), 30_000);
    write(&home.join("Library/Developer/Xcode/DerivedData/App-abc/x"), 30_000);
    // real projects: only their build folders are offered
    fake_project(&home.join("Vcity/aliexpress-scraper"));
    fake_project(&home.join("Downloads/27-shaders"));
    fake_project(&home.join("Documents/side/app"));
    fake_project(&home.join("code/app"));
    fs::create_dir_all(home.join("code/app/.git")).unwrap();
    // a node_modules without a package.json next to it is a global install
    write(&home.join("code/tools/lib/node_modules/x/index.js"), 20_000);

    for e in EXCLUDED {
        let p = home.join(e);
        assert!(protected(&p, Some(&home)), "{e} must be protected");
        assert!(protected(&p.join("node_modules"), Some(&home)), "{e}/node_modules must be protected");
    }
    for w in WHOLE {
        assert!(protected(&home.join(w), Some(&home)), "{w:?} must never be offered as a whole");
    }
    for ok in ["Downloads/27-shaders/node_modules", "code/app/target", "Documents/side/app/node_modules", ".cache/app-target", ".npm/_cacache", "Library/Developer/Xcode/DerivedData"] {
        assert!(!protected(&home.join(ok), Some(&home)), "{ok} is a regenerable build folder or cache");
    }
    // a stray `npm install` in the home folder: not a project's build folder
    write(&home.join("package.json"), 10);
    assert!(protected(&home.join("node_modules"), Some(&home)));
    for searched in ["", "Documents", "Downloads", "code"] {
        assert!(!protected_tree(&home.join(searched), Some(&home)), "{searched:?} is searched inside");
    }

    // the home folder, and protected folders given as search roots
    let roots: Vec<PathBuf> = std::iter::once(home.clone()).chain(EXCLUDED.iter().map(|e| home.join(e))).collect();
    let r = dev::find(&roots, Some(&home), &Progress::default());
    let candidates: Vec<&Path> = r.projects.iter().flat_map(|p| p.artifacts.iter().map(|a| a.path.as_path())).chain(r.caches.iter().map(|c| c.path.as_path())).collect();
    for c in &candidates {
        let rel = c.strip_prefix(&home).unwrap();
        let exact_cache = scan::safety::HOME_CACHE_ALLOW.iter().any(|a| rel == Path::new(a));
        for e in EXCLUDED.iter().filter(|_| !exact_cache) {
            assert!(!rel.starts_with(e), "{} offered from excluded {e}", rel.display());
        }
        assert!(!WHOLE.iter().any(|w| rel == Path::new(w)), "{} offered as a whole", rel.display());
        assert!(!dev::looks_like_project(c), "{} is a project, not a build folder", rel.display());
        assert!(!protected(c, Some(&home)));
    }
    let mut rels: Vec<String> = candidates.iter().map(|c| c.strip_prefix(&home).unwrap().display().to_string().replace('\\', "/")).collect();
    rels.sort();
    let mut want = vec![
        ".cache/app-target",
        ".npm/_cacache",
        "Library/Developer/Xcode/DerivedData",
        "Documents/side/app/node_modules",
        "Documents/side/app/target",
        "Downloads/27-shaders/node_modules",
        "Downloads/27-shaders/target",
        "Vcity/aliexpress-scraper/node_modules",
        "Vcity/aliexpress-scraper/target",
        "code/app/node_modules",
        "code/app/target",
    ];
    want.sort();
    assert_eq!(rels, want, "only build folders inside projects and exact caches");
    // projects are reported by their build folders; the folder itself is never a candidate
    for p in &r.projects {
        assert!(p.artifacts.iter().all(|a| a.path != p.root && a.path.starts_with(&p.root)));
    }

    // and the cleanup refuses them even when handed one directly
    let mut fake = r.caches[0].clone();
    for e in [".config/opencode/node_modules", ".vscode-server/cli/servers/Stable-abc/server/node_modules", ".cargo/registry", "Library/Mobile Documents/com~apple~CloudDocs/proj/target"] {
        fake.path = home.join(e);
        let got = dev::delete_with(&fake, None, Some(&home), |_| panic!("must not remove {e}"));
        assert_eq!(got, Err(Refusal::Protected), "{e}");
    }
    let mut proj = r.projects.iter().find(|p| p.root.ends_with("code/app")).unwrap().clone();
    let mut whole = proj.artifacts[0].clone();
    whole.path = proj.root.clone();
    proj.root = home.join("code");
    assert_eq!(dev::delete_with(&whole, Some(&proj), Some(&home), |_| panic!("never a whole project")), Err(Refusal::Protected));
    assert!(home.join(".config/opencode/node_modules/dep/index.js").exists());
}

#[cfg(unix)]
#[test]
fn system_and_bundles_are_protected_without_a_home() {
    for p in ["/System/Library/x", "/usr/local/lib/node_modules", "/opt/homebrew/lib/node_modules", "/Applications/Foo.app/Contents/Resources/app/node_modules", "/Library/Caches/x", "/private/var/x", "/", "/Users", "/Users/ana", "/Users/ana/.config/opencode/node_modules", "/Users/ana/Documents", "/Volumes/Backup/Users/ana/.vscode-server/x/node_modules", "/Volumes/Work/.ssh", "/Volumes/Work", "/Users/ana/node_modules", "relative/target"] {
        assert!(protected(Path::new(p), None), "{p}");
    }
    for p in ["/Volumes/Work/code/app/target", "/Users/ana/code/app/node_modules", "/Users/ana/Documents/app/node_modules", "/srv/code/app/target"] {
        assert!(!protected(Path::new(p), None), "{p}");
    }
}
