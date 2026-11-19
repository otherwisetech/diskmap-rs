//! `cargo run --release -p diskmap-scan --example du -- <dir>`: scan and print the top entries.
use scan::{Options, Progress, Tree, human, scan};
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let t0 = Instant::now();
    let p = Progress::default();
    let tree = match scan(dir.as_ref(), &Options::default(), &p) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{dir}: {e}");
            std::process::exit(1);
        }
    };
    let root = tree.node(Tree::ROOT);
    println!("{} in {} files, {} nodes, {} errors, {:?}", human(root.size), root.files, tree.len(), p.errors.load(std::sync::atomic::Ordering::Relaxed), t0.elapsed());
    for &c in root.children.iter().take(15) {
        let n = tree.node(c);
        println!("{:>10}  {}", human(n.size), n.name);
    }
}
