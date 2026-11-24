//! Disk usage scanning and treemap layout. No UI: the app renders what this crate computes.
//!
//! - [`walk::scan`] walks a directory in parallel into a [`Tree`] (children sorted by size).
//! - [`treemap::layout`] turns a subtree into nested squarified [`treemap::Tile`]s.
pub mod dev;
pub mod kind;
pub mod mcp;
pub mod safety;
pub mod snapshot;
pub mod search;
pub mod treemap;
pub mod tree;
pub mod walk;

pub use kind::Kind;
pub use tree::{Node, NodeId, Tree};
pub use walk::{Options, Progress, scan};

/// Human-readable size in SI units (as Finder / GNOME show them): `0 B`, `840 B`, `1.2 kB`, `34 MB`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 999.95 && u < UNITS.len() - 1 {
        v /= 1000.;
        u += 1;
    }
    if v < 99.95 { format!("{v:.1} {}", UNITS[u]) } else { format!("{v:.0} {}", UNITS[u]) }
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(999), "999 B");
        assert_eq!(human(1000), "1.0 kB");
        assert_eq!(human(1_234_567), "1.2 MB");
        assert_eq!(human(345_000_000), "345 MB");
        assert_eq!(human(999_999_999), "1.0 GB");
        assert_eq!(human(2_500_000_000_000), "2.5 TB");
    }
}
