//! otherwise-link: `otherwise-diskmap://` links in, other Otherwise apps in About, low-disk-space
//! alerts in Otherwise Apps' Today view (opening Developer projects when build folders are a big
//! part of the volume), and rama: "Open in rama" and the worktrees it knows.
use otherwise_link::apps::{self, AppSpec, Installed, Locations};
use otherwise_link::today::{self, Event, EventKind, Severity};
use otherwise_link::{Link, Root};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const APP_ID: &str = "diskmap";
#[cfg(target_os = "linux")]
const BUNDLE_ID: &str = "io.github.otherwisetech.diskmap";
/// Below this share of free space a volume shows up in Today.
const LOW_SPACE: f64 = 0.10;
const CRITICAL_SPACE: f64 = 0.03;
const TODAY_EVERY: Duration = Duration::from_secs(10 * 60);

/// What an incoming link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bring the window to the front.
    Focus,
    Scan(PathBuf),
    /// Open Developer projects.
    Dev,
}

/// Build folders worth this share of a volume's used space make its low-space alert open
/// Developer projects instead of a plain scan.
const DEV_SHARE: f64 = 0.10;

/// `otherwise://diskmap/scan?path=/Volumes/X` → scan it; anything else of ours → focus.
/// Relative paths are ignored (they would resolve against whatever folder we started in).
pub fn action(link: &Link) -> Option<Action> {
    if link.app != APP_ID {
        return None;
    }
    if link.at(0) == Some("dev") {
        return Some(Action::Dev);
    }
    if link.at(0) == Some("scan")
        && let Some(p) = link.get("path").map(PathBuf::from)
        && p.is_absolute()
    {
        return Some(Action::Scan(p));
    }
    Some(Action::Focus)
}

/// `otherwise://diskmap/scan?path=…`
pub fn scan_link(path: &Path) -> Link {
    Link::new(APP_ID).segment("scan").param("path", path.to_string_lossy())
}

/// `otherwise://diskmap/dev`
pub fn dev_link() -> Link {
    Link::new(APP_ID).segment("dev")
}

const RAMA: &str = "rama"; // i18n-ok: app id

/// `otherwise://rama/repo?path=…`
pub fn rama_link(repo: &Path) -> Link {
    Link::new(RAMA).segment("repo").param("path", repo.to_string_lossy())
}

pub fn rama_installed() -> bool {
    apps::known(RAMA).is_some_and(|k| apps::detect(&k.spec(), &Locations::system()).is_some())
}

pub fn open_in_rama(repo: &Path) -> std::io::Result<()> {
    otherwise_link::open::open_link(&rama_link(repo)).map(|_| ())
}

/// Worktree folders rama has listed (`<data root>/rama/worktrees.txt`, one path per line).
pub fn rama_worktrees(file: &Path) -> std::collections::HashSet<PathBuf> {
    std::fs::read_to_string(file).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

pub fn rama_worktrees_file() -> PathBuf {
    otherwise_link::paths::app_dir(RAMA).join("worktrees.txt")
}

/// The last Developer projects scan: where, how much build output, when.
#[derive(Debug, Clone, PartialEq)]
pub struct DevSummary {
    pub root: PathBuf,
    pub bytes: u64,
    pub at: i64,
}

impl DevSummary {
    pub fn to_json(&self) -> String {
        serde_json::json!({ "root": self.root.to_string_lossy(), "bytes": self.bytes, "at": self.at }).to_string()
    }

    pub fn parse(s: &str) -> Option<DevSummary> {
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        Some(DevSummary { root: PathBuf::from(v.get("root")?.as_str()?), bytes: v.get("bytes")?.as_u64()?, at: v.get("at")?.as_i64()? })
    }

    fn file() -> PathBuf {
        crate::storage::app_dir().join("dev-summary.json")
    }

    pub fn load() -> Option<DevSummary> {
        DevSummary::parse(&std::fs::read_to_string(DevSummary::file()).ok()?)
    }

    pub fn save(&self) {
        let f = DevSummary::file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, self.to_json());
    }
}

/// Windows and Linux learn about our scheme at run time (macOS from Info.plist).
pub fn register_scheme() {
    std::thread::spawn(|| {
        #[cfg(windows)]
        let _ = crate::explorer::register_scheme(&otherwise_link::link::app_scheme(APP_ID));
        #[cfg(target_os = "linux")]
        {
            let apps = otherwise_link::paths::home().join(".local/share/applications");
            for file in ["diskmap.desktop".to_string(), format!("{BUNDLE_ID}.desktop")] { // i18n-ok: file names
                if apps.join(&file).is_file() {
                    let _ = otherwise_link::register::register_linux(&file, &[&otherwise_link::link::app_scheme(APP_ID)]);
                    break;
                }
            }
        }
    });
}

/// Other Otherwise apps installed on this computer with a window to open (not CLI-only), by name.
pub fn other_apps(loc: &Locations) -> Vec<(AppSpec, Installed)> {
    let mut found: Vec<_> = apps::discover(loc).into_iter().filter(|(s, i)| s.id != APP_ID && (i.path.is_some() || i.executable.is_some())).collect();
    found.sort_by_key(|(s, _)| s.name.to_lowercase());
    found
}

/// Starts an installed app: the bundle on macOS, its executable elsewhere.
pub fn launch(app: &Installed) -> std::io::Result<()> {
    if cfg!(target_os = "macos")
        && let Some(p) = &app.path
    {
        return std::process::Command::new("open").arg(p).status().map(|_| ());
    }
    let exe = app.executable.as_ref().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    std::process::Command::new(exe).spawn().map(|_| ())
}

/// One mounted volume as far as Today cares.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub mount: PathBuf,
    pub name: String,
    pub total: u64,
    pub free: u64,
}

/// Mounted volumes worth watching: no pseudo / system / read-only image file systems, no
/// macOS system volumes (they share the Data volume's space), no boot partitions.
pub fn volumes() -> Vec<Volume> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Volume> = Vec::new();
    for d in disks.list() {
        let fs = d.file_system().to_string_lossy().to_lowercase();
        let mount = d.mount_point().to_path_buf();
        if !watched(&mount, &fs, d.total_space()) {
            continue;
        }
        let name = d.name().to_string_lossy().trim().to_string();
        let name = if name.is_empty() || name.starts_with("/dev/") { mount.display().to_string() } else { name };
        if out.iter().any(|v| v.mount == mount) {
            continue;
        }
        out.push(Volume { mount, name, total: d.total_space(), free: d.available_space() });
    }
    out
}

fn watched(mount: &Path, fs: &str, total: u64) -> bool {
    const SKIP_FS: [&str; 12] = ["tmpfs", "devtmpfs", "devfs", "autofs", "overlay", "squashfs", "proc", "sysfs", "nullfs", "ramfs", "iso9660", "udf"]; // i18n-ok: file system names
    const SKIP_UNDER: [&str; 6] = ["/System/Volumes", "/private/var/vm", "/boot", "/snap", "/run/user", "/dev"]; // i18n-ok: paths
    total > 0 && !SKIP_FS.contains(&fs) && !SKIP_UNDER.iter().any(|p| mount.starts_with(p))
}

/// Stable id per volume: the mount point, reduced to `[a-z0-9-]`.
fn event_id(mount: &Path) -> String {
    let s: String = mount.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-');
    format!("low-space-{}", if s.is_empty() { "root" } else { s }) // i18n-ok: event id
}

/// The volume holding `path`: the one with the longest mount point above it.
fn volume_of<'a>(volumes: &'a [Volume], path: &Path) -> Option<&'a Volume> {
    volumes.iter().filter(|v| path.starts_with(&v.mount)).max_by_key(|v| v.mount.components().count())
}

/// Today events for volumes under 10 % free (in the current language). They expire tomorrow,
/// so a feed left behind by a diskmap that isn't running anymore goes away by itself. When the
/// last Developer projects scan found build output worth a tenth of the volume's used space,
/// the alert says so and opens that view.
pub fn low_space_events(volumes: &[Volume], today: chrono::NaiveDate, dev: Option<&DevSummary>) -> Vec<Event> {
    let dev_volume = dev.and_then(|d| volume_of(volumes, &d.root)).map(|v| v.mount.clone());
    volumes
        .iter()
        .filter(|v| v.total > 0 && (v.free as f64) < v.total as f64 * LOW_SPACE)
        .map(|v| {
            let share = v.free as f64 / v.total as f64;
            let pct = format!("{:.0}", (share * 100.).floor());
            let used = v.total.saturating_sub(v.free) as f64;
            let dev = dev.filter(|d| dev_volume.as_ref() == Some(&v.mount) && d.bytes as f64 >= used * DEV_SHARE);
            let (body, link) = match dev {
                Some(d) => (i18n::t!("app.today.low_space_dev_body", free = scan::human(v.free), total = scan::human(v.total), dev = scan::human(d.bytes)), dev_link()),
                None => (i18n::t!("app.today.low_space_body", free = scan::human(v.free), total = scan::human(v.total)), scan_link(&v.mount)),
            };
            Event::new(event_id(&v.mount), EventKind::Alert, i18n::t!("app.today.low_space", volume = v.name, pct = pct))
                .body(body)
                .severity(if share < CRITICAL_SPACE { Severity::Critical } else { Severity::High })
                .link(&link)
                .expires((today + chrono::Days::new(1)).to_string())
        })
        .collect()
}

pub fn publish_today(root: &Root) -> std::io::Result<()> {
    today::publish(root, APP_ID, &low_space_events(&volumes(), chrono::Local::now().date_naive(), DevSummary::load().as_ref()))
}

/// Republishes the Today feed now and every few minutes, off the UI thread.
pub fn watch_today() {
    std::thread::spawn(|| {
        loop {
            let _ = publish_today(&Root::system());
            std::thread::sleep(TODAY_EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_links_round_trip() {
        let p = if cfg!(windows) { PathBuf::from(r"C:\Users\a b") } else { PathBuf::from("/Volumes/Datos ñ") };
        let link = scan_link(&p);
        assert!(link.to_url().starts_with("otherwise://diskmap/scan?path="));
        assert_eq!(action(&Link::parse(&link.to_url()).unwrap()), Some(Action::Scan(p.clone())));
        assert_eq!(action(&Link::parse(&link.app_url()).unwrap()), Some(Action::Scan(p)));
    }

    #[test]
    fn other_links_focus_or_are_not_ours() {
        let a = |s: &str| action(&Link::parse(s).unwrap());
        assert_eq!(a("otherwise://diskmap"), Some(Action::Focus));
        assert_eq!(a("otherwise-diskmap://"), Some(Action::Focus));
        assert_eq!(a("otherwise://diskmap/entity/6f0c2c1e-0000-4000-8000-000000000001"), Some(Action::Focus));
        assert_eq!(a("otherwise://diskmap/scan?path=relative/dir"), Some(Action::Focus));
        assert_eq!(a("otherwise://diskmap/scan"), Some(Action::Focus));
        assert_eq!(a("otherwise://rama/repo?path=/x"), None);
    }

    fn vol(mount: &str, total: u64, free: u64) -> Volume {
        Volume { mount: PathBuf::from(mount), name: mount.into(), total, free }
    }

    #[test]
    fn only_low_volumes_become_alerts() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let ev = low_space_events(&[vol("/", 1000, 95), vol("/Volumes/Big", 1000, 500), vol("/Volumes/Full", 1000, 10), vol("/Volumes/Zero", 0, 0)], day, None);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].id, "low-space-root");
        assert_eq!(ev[0].severity, Severity::High);
        assert_eq!(ev[0].kind, EventKind::Alert);
        assert_eq!(ev[0].link.as_deref(), Some("otherwise://diskmap/scan?path=%2F"));
        assert_eq!(ev[0].expires.as_deref(), Some("2026-11-22"));
        assert!(ev[0].title.contains("9"), "{}", ev[0].title);
        assert_eq!(ev[1].id, "low-space-volumes-full");
        assert_eq!(ev[1].severity, Severity::Critical);
    }

    #[test]
    fn low_space_opens_dev_projects_when_build_folders_are_a_big_share() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let vols = [vol("/", 1000, 50), vol("/Volumes/Ext", 1000, 20)];
        let home = PathBuf::from("/Users/ana");
        // 950 used on /: 100 of build output is more than a tenth
        let big = DevSummary { root: home.clone(), bytes: 100, at: 1 };
        let ev = low_space_events(&vols, day, Some(&big));
        assert_eq!(ev[0].link.as_deref(), Some("otherwise://diskmap/dev"));
        assert!(ev[0].body.as_deref().unwrap_or_default().contains("100 B"), "{:?}", ev[0].body);
        // the other volume doesn't hold the projects: plain scan link
        assert_eq!(ev[1].link.as_deref(), Some("otherwise://diskmap/scan?path=%2FVolumes%2FExt"));
        let small = DevSummary { bytes: 50, ..big.clone() };
        assert_eq!(low_space_events(&vols, day, Some(&small))[0].link.as_deref(), Some("otherwise://diskmap/scan?path=%2F"));
        let on_ext = DevSummary { root: PathBuf::from("/Volumes/Ext/code"), ..big.clone() };
        let ev = low_space_events(&vols, day, Some(&on_ext));
        assert_eq!((ev[0].link.as_deref(), ev[1].link.as_deref()), (Some("otherwise://diskmap/scan?path=%2F"), Some("otherwise://diskmap/dev")));
        assert_eq!(DevSummary::parse(&big.to_json()), Some(big));
        assert_eq!(DevSummary::parse("nope"), None);
    }

    #[test]
    fn dev_and_rama_links() {
        assert_eq!(action(&Link::parse(&dev_link().to_url()).unwrap()), Some(Action::Dev));
        assert_eq!(action(&Link::parse("otherwise-diskmap://dev").unwrap()), Some(Action::Dev));
        let repo = if cfg!(windows) { PathBuf::from(r"C:\code\app ñ") } else { PathBuf::from("/code/app ñ") };
        let l = rama_link(&repo);
        assert!(l.to_url().starts_with("otherwise://rama/repo?path="), "{}", l.to_url());
        let back = Link::parse(&l.to_url()).unwrap();
        assert_eq!((back.app.as_str(), back.at(0), back.get("path").map(PathBuf::from)), ("rama", Some("repo"), Some(repo)));
        assert_eq!(action(&back), None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("worktrees.txt");
        std::fs::write(&f, "/a/wt\n\n/b/main\n").unwrap();
        let known = rama_worktrees(&f);
        assert!(known.contains(Path::new("/a/wt")) && known.contains(Path::new("/b/main")) && known.len() == 2);
        assert!(rama_worktrees(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn publishes_and_clears_the_feed() {
        let dir = tempfile::tempdir().unwrap();
        let root = Root::at(dir.path());
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        today::publish(&root, APP_ID, &low_space_events(&[vol("/", 100, 1)], day, None)).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-21T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let merged = today::read_in(&root, now, &chrono::Utc);
        assert_eq!(merged.events.len(), 1);
        assert_eq!(merged.events[0].app, APP_ID);
        today::publish(&root, APP_ID, &low_space_events(&[vol("/", 100, 50)], day, None)).unwrap();
        assert!(today::read_in(&root, now, &chrono::Utc).events.is_empty());
    }

    #[test]
    fn skips_pseudo_and_system_volumes() {
        assert!(watched(Path::new("/"), "apfs", 10));
        assert!(watched(Path::new("/Volumes/USB"), "exfat", 10));
        assert!(!watched(Path::new("/System/Volumes/Data"), "apfs", 10));
        assert!(!watched(Path::new("/boot/efi"), "vfat", 10));
        assert!(!watched(Path::new("/run"), "tmpfs", 10));
        assert!(!watched(Path::new("/x"), "ext4", 0));
    }

    #[test]
    fn about_lists_other_installed_apps_only() {
        let dir = tempfile::tempdir().unwrap();
        let (opt, bin) = (dir.path().join("opt"), dir.path().join("bin"));
        for (folder, exe) in [("diskmap", "diskmap"), ("vigia", "vigia-app"), ("genealogy-rs", "genealogy-rs")] {
            std::fs::create_dir_all(opt.join(folder)).unwrap();
            std::fs::write(opt.join(folder).join(exe), "").unwrap();
        }
        // a CLI-only install has no window to open
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("nodo"), "").unwrap();
        let loc = Locations { os: apps::Os::Linux, mac_dirs: vec![], roots: vec![opt], path: vec![bin] };
        let ids: Vec<String> = other_apps(&loc).into_iter().map(|(s, _)| s.id).collect();
        assert_eq!(ids, ["genealogy", "vigia"]);
    }
}
