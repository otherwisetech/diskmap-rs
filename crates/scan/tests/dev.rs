//! Developer projects: build folders found and grouped, caches, and the deletion safety rules.
use scan::Progress;
use scan::dev::{self, ArtifactKind, Refusal};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").args(["-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"]).args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", empty_config()).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn empty_config() -> PathBuf {
    let f = std::env::temp_dir().join(format!("diskmap-dev-test-gitconfig-{}", std::process::id()));
    if !f.exists() {
        fs::write(&f, "").unwrap();
    }
    f
}

/// The cleanup with a plain remove: tests never touch the real Trash.
fn del(a: &dev::Artifact, p: Option<&dev::Project>, home: Option<&Path>) -> Result<(), Refusal> {
    dev::delete_with(a, p, home, |x| fs::remove_dir_all(x).map_err(|e| e.to_string()))
}

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = dev::canonical(d.path()).unwrap();
    (d, p)
}

/// home/code/app (git, cargo + node), home/code/app/web (inside it), home/code/loose (no git),
/// home/code/notes (no build folders), caches in home.
fn sample(home: &Path) {
    let app = home.join("code/app");
    write(&app.join("Cargo.toml"), 10);
    write(&app.join("src/main.rs"), 1_000);
    write(&app.join("target/CACHEDIR.TAG"), 10);
    write(&app.join("target/debug/big.rlib"), 200_000);
    write(&app.join("web/package.json"), 10);
    write(&app.join("web/node_modules/a/index.js"), 50_000);
    write(&app.join("web/.next/cache.bin"), 30_000);
    write(&home.join("code/loose/build.gradle"), 10);
    write(&home.join("code/loose/build/out.class"), 40_000);
    write(&home.join("code/loose/.gradle/x.bin"), 5_000);
    write(&home.join("code/notes/build/keep.txt"), 9_000);
    write(&home.join("code/notes/target/keep.txt"), 9_000);
    write(&home.join(".cargo/registry/cache/x.crate"), 70_000);
    write(&home.join(".npm/_cacache/index/x"), 60_000);
    write(&home.join(".cache/app-target/CACHEDIR.TAG"), 10);
    write(&home.join(".cache/app-target/debug/x"), 80_000);
    write(&home.join(".cache/not-a-target/x"), 80_000);
    // inside skipped folders: never searched
    write(&home.join("Library/stuff/node_modules/x.js"), 10_000);
}

#[test]
fn finds_build_folders_grouped_by_project() {
    let (_d, home) = tmp();
    sample(&home);
    git(&home.join("code/app"), &["init", "-q"]);
    let r = dev::find(std::slice::from_ref(&home), Some(&home), &Progress::default());
    let roots: Vec<&Path> = r.projects.iter().map(|p| p.root.as_path()).collect();
    assert_eq!(roots, [home.join("code/app"), home.join("code/loose")]);
    let app = &r.projects[0];
    assert!(app.git && !app.worktree);
    let kinds: Vec<(ArtifactKind, PathBuf)> = app.artifacts.iter().map(|a| (a.kind, a.path.clone())).collect();
    assert_eq!(kinds, [(ArtifactKind::Target, home.join("code/app/target")), (ArtifactKind::NodeModules, home.join("code/app/web/node_modules")), (ArtifactKind::Next, home.join("code/app/web/.next"))]);
    assert!(app.bytes >= 280_000 && app.modified > 0);
    let loose = &r.projects[1];
    assert!(!loose.git);
    assert_eq!(loose.artifacts.iter().map(|a| a.kind).collect::<Vec<_>>(), [ArtifactKind::GradleBuild, ArtifactKind::Gradle]);
    let caches: Vec<(ArtifactKind, PathBuf)> = r.caches.iter().map(|c| (c.kind, c.path.clone())).collect();
    // ~/.cargo holds binaries and settings next to its registry: never offered
    assert_eq!(caches, [(ArtifactKind::StandaloneTarget, home.join(".cache/app-target")), (ArtifactKind::NpmCache, home.join(".npm/_cacache"))]);
    assert!(r.total() > r.build_bytes() && r.build_bytes() >= app.bytes + loose.bytes);
    // a root inside another is searched once; the same report comes back
    let again = dev::find(&[home.join("code"), home.clone()], Some(&home), &Progress::default());
    assert_eq!(again, r);
}

#[test]
fn worktrees_are_projects_of_their_own() {
    let (_d, home) = tmp();
    let main = home.join("main");
    write(&main.join("Cargo.toml"), 10);
    git(&main, &["init", "-q"]);
    git(&main, &["add", "."]);
    git(&main, &["commit", "-q", "-m", "x"]);
    git(&main, &["worktree", "add", "-q", home.join("main/.worktrees/wt").to_str().unwrap()]);
    write(&main.join("target/CACHEDIR.TAG"), 10);
    write(&main.join(".worktrees/wt/target/CACHEDIR.TAG"), 10);
    write(&main.join(".worktrees/wt/target/x"), 20_000);
    let r = dev::find(std::slice::from_ref(&home), Some(&home), &Progress::default());
    let wt = r.projects.iter().find(|p| p.root.ends_with("wt")).unwrap();
    assert!(wt.git && wt.worktree);
    let m = r.projects.iter().find(|p| p.root == main).unwrap();
    assert!(m.artifacts.iter().all(|a| !a.path.starts_with(&wt.root)));
    assert!(r.caches.is_empty());
}

#[test]
fn delete_refuses_anything_but_ignored_build_folders() {
    let (_d, home) = tmp();
    sample(&home);
    let app = home.join("code/app");
    fs::write(app.join(".gitignore"), "target/\n").unwrap();
    git(&app, &["init", "-q"]);
    git(&app, &["add", ".gitignore", "Cargo.toml", "src"]);
    git(&app, &["commit", "-q", "-m", "x"]);
    let r = dev::find(std::slice::from_ref(&home), Some(&home), &Progress::default());
    let p = r.projects.iter().find(|p| p.root == app).unwrap().clone();
    let art = |k: ArtifactKind| p.artifacts.iter().find(|a| a.kind == k).unwrap().clone();
    // node_modules isn't ignored here: git sees it, so it stays
    assert_eq!(del(&art(ArtifactKind::NodeModules), Some(&p), Some(&home)), Err(Refusal::Changed));
    assert!(app.join("web/node_modules/a/index.js").exists());
    // a path that isn't a build folder of that kind, or outside the project
    let mut fake = art(ArtifactKind::Target);
    fake.path = app.join("src");
    assert_eq!(del(&fake, Some(&p), Some(&home)), Err(Refusal::NotArtifact));
    let mut outside = art(ArtifactKind::Target);
    outside.path = home.join("code/notes/target");
    assert_eq!(del(&outside, Some(&p), Some(&home)), Err(Refusal::NotArtifact));
    let mut wrong_kind = art(ArtifactKind::Target);
    wrong_kind.kind = ArtifactKind::NodeModules;
    assert_eq!(del(&wrong_kind, Some(&p), Some(&home)), Err(Refusal::NotArtifact));
    #[cfg(unix)]
    {
        // a symlink where the folder was is never followed
        let t = art(ArtifactKind::Target);
        fs::rename(&t.path, app.join("moved")).unwrap();
        std::os::unix::fs::symlink(app.join("moved"), &t.path).unwrap();
        assert_eq!(del(&t, Some(&p), Some(&home)), Err(Refusal::NotArtifact));
        assert!(app.join("moved/debug/big.rlib").exists());
        fs::remove_file(&t.path).unwrap();
        fs::rename(app.join("moved"), &t.path).unwrap();
    }
    // ignored target/: goes, sources stay
    del(&art(ArtifactKind::Target), Some(&p), Some(&home)).unwrap();
    assert!(!app.join("target").exists() && app.join("src/main.rs").exists() && app.join("Cargo.toml").exists());
    // a committed build folder stays even when ignored
    fs::write(app.join(".gitignore"), "target/\nnode_modules/\n.next/\n").unwrap();
    git(&app, &["add", "-f", "web/.next"]);
    git(&app, &["commit", "-q", "-m", "next"]);
    assert_eq!(del(&art(ArtifactKind::Next), Some(&p), Some(&home)), Err(Refusal::Changed));
    del(&art(ArtifactKind::NodeModules), Some(&p), Some(&home)).unwrap();

    // non-git project: name rules only
    let loose = r.projects.iter().find(|p| !p.git).unwrap();
    del(&loose.artifacts[0], Some(loose), Some(&home)).unwrap();
    assert!(home.join("code/loose/build.gradle").exists());

    // caches: only the exact known folders
    let reg = r.caches.iter().find(|c| c.kind == ArtifactKind::NpmCache).unwrap();
    let mut other = reg.clone();
    other.path = home.join(".cache/not-a-target");
    // hidden home folders other than the exact caches are protected; without a home, all are
    assert_eq!(del(&other, None, Some(&home)), Err(Refusal::Protected));
    assert_eq!(del(reg, None, None), Err(Refusal::Protected));
    del(reg, None, Some(&home)).unwrap();
    assert!(!home.join(".npm/_cacache").exists() && home.join(".cargo/registry/cache/x.crate").exists());
    // ~/.cargo/registry passed off as a cache is refused
    let mut cargo = reg.clone();
    cargo.path = home.join(".cargo/registry");
    assert_eq!(del(&cargo, None, Some(&home)), Err(Refusal::Protected));
    let st = r.caches.iter().find(|c| c.kind == ArtifactKind::StandaloneTarget).unwrap();
    del(st, None, Some(&home)).unwrap();
    assert!(home.join(".cache/not-a-target/x").exists());
}

#[test]
fn classify_needs_the_files_next_to_it() {
    let (_d, p) = tmp();
    for d in ["a/target", "b/target", "c/.next", "d/build", "e/build", "f/.gradle", "g/node_modules", "h/node_modules"] {
        fs::create_dir_all(p.join(d)).unwrap();
    }
    write(&p.join("a/Cargo.toml"), 1);
    write(&p.join("c/package.json"), 1);
    write(&p.join("d/build.gradle.kts"), 1);
    write(&p.join("f/settings.gradle"), 1);
    write(&p.join("h/package.json"), 1);
    let k = |d: &str| dev::classify(&p.join(d));
    assert_eq!(k("a/target"), Some(ArtifactKind::Target));
    assert_eq!(k("b/target"), None);
    assert_eq!(k("c/.next"), Some(ArtifactKind::Next));
    assert_eq!(k("d/build"), Some(ArtifactKind::GradleBuild));
    assert_eq!(k("e/build"), None);
    assert_eq!(k("f/.gradle"), Some(ArtifactKind::Gradle));
    assert_eq!(k("g/node_modules"), None, "a global install, not a project's");
    assert_eq!(k("h/node_modules"), Some(ArtifactKind::NodeModules));
}
