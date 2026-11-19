use scan::treemap::{LayoutOptions, Rect, hit, layout, squarify};
use scan::{Kind, Options, Progress, Tree, scan};
use std::fs;
use std::path::Path;

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

fn alloc(p: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::symlink_metadata(p).unwrap().blocks() * 512
    }
    #[cfg(not(unix))]
    fs::symlink_metadata(p).unwrap().len()
}

fn sample() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(&r.join("big/movie.mp4"), 300_000);
    write(&r.join("big/inner/photo.jpg"), 120_000);
    write(&r.join("small/notes.txt"), 10);
    write(&r.join("readme.md"), 50_000);
    fs::create_dir(r.join("empty")).unwrap();
    d
}

fn find(t: &Tree, name: &str) -> u32 {
    (0..t.len() as u32).find(|&i| &*t.node(i).name == name).unwrap_or_else(|| panic!("{name} not in tree"))
}

#[test]
fn totals_sorting_and_paths() {
    let d = sample();
    let p = Progress::default();
    let t = scan(d.path(), &Options::default(), &p).unwrap();
    let root = t.node(Tree::ROOT);
    assert_eq!(root.files, 4);
    let files: u64 = ["big/movie.mp4", "big/inner/photo.jpg", "small/notes.txt", "readme.md"].iter().map(|f| alloc(&d.path().join(f))).sum();
    assert!(root.size >= files, "root {} < files {files}", root.size);
    assert_eq!(p.files.load(std::sync::atomic::Ordering::Relaxed), 4);
    // children sorted by size, largest first
    let sizes: Vec<u64> = root.children.iter().map(|&c| t.node(c).size).collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    assert_eq!(&*t.node(root.children[0]).name, "big");
    let photo = find(&t, "photo.jpg");
    assert_eq!(t.node(photo).kind, Kind::Image);
    assert_eq!(t.path(photo), fs::canonicalize(d.path()).unwrap().join("big").join("inner").join("photo.jpg"));
    assert_eq!(t.ancestors(photo).len(), 4);
    assert!(t.is_ancestor(find(&t, "big"), photo));
    assert!(!t.is_ancestor(find(&t, "small"), photo));
    assert_eq!(t.largest_files(Tree::ROOT, 2), vec![find(&t, "movie.mp4"), find(&t, "photo.jpg")]);
}

#[test]
fn remove_updates_ancestors() {
    let d = sample();
    let mut t = scan(d.path(), &Options::default(), &Progress::default()).unwrap();
    let before = t.node(Tree::ROOT).size;
    let movie = find(&t, "movie.mp4");
    let msize = t.node(movie).size;
    let big = find(&t, "big");
    let big_before = t.node(big).size;
    t.remove(movie);
    assert_eq!(t.node(Tree::ROOT).size, before - msize);
    assert_eq!(t.node(big).size, big_before - msize);
    assert_eq!(t.node(Tree::ROOT).files, 3);
    assert!(!t.node(big).children.contains(&movie));
    t.remove(movie); // twice is harmless
    assert_eq!(t.node(Tree::ROOT).size, before - msize);
    let kids: Vec<u64> = t.node(Tree::ROOT).children.iter().map(|&c| t.node(c).size).collect();
    assert!(kids.windows(2).all(|w| w[0] >= w[1]));
}

#[cfg(unix)]
#[test]
fn hard_links_count_once_and_symlinks_are_not_followed() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join("a/data.bin"), 200_000);
    fs::create_dir(d.path().join("b")).unwrap();
    fs::hard_link(d.path().join("a/data.bin"), d.path().join("b/same.bin")).unwrap();
    std::os::unix::fs::symlink(d.path().join("a"), d.path().join("loop")).unwrap();
    let t = scan(d.path(), &Options::default(), &Progress::default()).unwrap();
    let one = alloc(&d.path().join("a/data.bin"));
    let data = t.node(find(&t, "data.bin")).size + t.node(find(&t, "same.bin")).size;
    assert_eq!(data, one);
    assert!(!t.node(find(&t, "loop")).is_dir());
    assert_eq!(t.node(Tree::ROOT).files, 3);
}

#[test]
fn cancel_and_bad_root() {
    let d = sample();
    let p = Progress::default();
    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let e = scan(d.path(), &Options::default(), &p).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::Interrupted);
    assert!(scan(&d.path().join("nope"), &Options::default(), &Progress::default()).is_err());
    assert!(scan(&d.path().join("readme.md"), &Options::default(), &Progress::default()).is_err());
}

#[test]
fn skip_list() {
    let d = sample();
    let opts = Options { skip: vec![fs::canonicalize(d.path()).unwrap().join("big")], ..Options::default() };
    let t = scan(d.path(), &opts, &Progress::default()).unwrap();
    assert_eq!(t.node(Tree::ROOT).files, 2);
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    let e = 1e-3;
    a.x + e < b.x + b.w && b.x + e < a.x + a.w && a.y + e < b.y + b.h && b.y + e < a.y + a.h
}

#[test]
fn squarify_fills_exactly_without_overlap() {
    let weights = [60., 30., 20., 10., 6., 4., 2., 1., 0.];
    let r = Rect::new(10., 20., 600., 400.);
    let rects = squarify(&weights, r);
    let total: f64 = weights.iter().sum();
    for (w, rc) in weights.iter().zip(&rects) {
        let expect = (*w / total) as f32 * r.area();
        assert!((rc.area() - expect).abs() < 0.5, "{rc:?} area {} vs {expect}", rc.area());
        if *w > 0. {
            assert!(rc.x >= r.x - 1e-3 && rc.y >= r.y - 1e-3 && rc.x + rc.w <= r.x + r.w + 1e-3 && rc.y + rc.h <= r.y + r.h + 1e-3, "{rc:?} outside");
        }
    }
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            assert!(!overlap(&rects[i], &rects[j]), "{:?} overlaps {:?}", rects[i], rects[j]);
        }
    }
    // squarified tiles stay reasonably square
    let worst = rects.iter().filter(|r| r.area() > 0.).map(|r| (r.w / r.h).max(r.h / r.w)).fold(0., f32::max);
    assert!(worst < 4., "worst aspect {worst}");
    assert!(squarify(&[0., 0.], r).iter().all(|r| r.area() == 0.));
}

#[test]
fn nested_layout_and_hit_test() {
    let d = sample();
    let t = scan(d.path(), &Options::default(), &Progress::default()).unwrap();
    let area = Rect::new(0., 0., 800., 600.);
    let tiles = layout(&t, Tree::ROOT, area, &LayoutOptions::default());
    let big = find(&t, "big");
    let movie = find(&t, "movie.mp4");
    let bt = tiles.iter().find(|x| x.node == big).unwrap();
    let mt = tiles.iter().find(|x| x.node == movie).unwrap();
    assert!(bt.nested && bt.depth == 0 && mt.depth == 1);
    // parents come before children
    assert!(tiles.iter().position(|x| x.node == big) < tiles.iter().position(|x| x.node == movie));
    let (cx, cy) = (mt.rect.x + mt.rect.w / 2., mt.rect.y + mt.rect.h / 2.);
    assert_eq!(hit(&tiles, cx, cy).unwrap().node, movie);
    assert_eq!(hit(&tiles, bt.rect.x + 1., bt.rect.y + 1.).unwrap().node, big);
    assert!(hit(&tiles, -5., -5.).is_none());
    // the empty directory has no allocated size worth a tile, or is a leaf
    assert!(tiles.iter().all(|x| !x.nested || !t.node(x.node).children.is_empty()));
}
