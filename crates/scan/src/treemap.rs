//! Squarified treemap (Bruls, Huizing, van Wijk 2000), nested by directory.
use crate::{NodeId, Tree};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn area(&self) -> f32 {
        self.w * self.h
    }
}

/// Split `r` into one rect per weight (sorted largest first), areas proportional to the weights.
/// Zero weights get an empty rect.
pub fn squarify(weights: &[f64], r: Rect) -> Vec<Rect> {
    let n = weights.len();
    let mut out = vec![Rect::default(); n];
    let total: f64 = weights.iter().sum();
    if total <= 0. || r.w <= 0. || r.h <= 0. {
        return out;
    }
    let scale = r.w as f64 * r.h as f64 / total;
    let a: Vec<f64> = weights.iter().map(|w| w * scale).collect();
    let (mut x, mut y, mut w, mut h) = (r.x as f64, r.y as f64, r.w as f64, r.h as f64);
    let mut i = 0;
    while i < n && a[i] > 0. {
        let side = w.min(h);
        let mut sum = a[i];
        let mut j = i + 1;
        let mut cur = worst(a[i], a[i], sum, side);
        while j < n && a[j] > 0. {
            let next = worst(a[i], a[j], sum + a[j], side);
            if next > cur {
                break;
            }
            cur = next;
            sum += a[j];
            j += 1;
        }
        // the last row takes whatever is left, absorbing float drift
        let last = j >= n || a[j] <= 0.;
        if w >= h {
            let thick = if last { w } else { sum / h };
            let mut yy = y;
            for k in i..j {
                let hh = if k + 1 == j { y + h - yy } else { a[k] / thick };
                out[k] = Rect::new(x as f32, yy as f32, thick as f32, hh as f32);
                yy += hh;
            }
            x += thick;
            w -= thick;
        } else {
            let thick = if last { h } else { sum / w };
            let mut xx = x;
            for k in i..j {
                let ww = if k + 1 == j { x + w - xx } else { a[k] / thick };
                out[k] = Rect::new(xx as f32, y as f32, ww as f32, thick as f32);
                xx += ww;
            }
            y += thick;
            h -= thick;
        }
        i = j;
    }
    out
}

/// Worst aspect ratio of a row with largest item `max`, smallest `min` and total `sum`.
fn worst(max: f64, min: f64, sum: f64, side: f64) -> f64 {
    let s2 = side * side;
    let sum2 = sum * sum;
    (s2 * max / sum2).max(sum2 / (s2 * min))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub node: NodeId,
    pub rect: Rect,
    /// 0 for the children of the layout root.
    pub depth: u16,
    /// Directory tile whose children are drawn inside it.
    pub nested: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    pub max_depth: u16,
    /// Header strip above a nested directory's children (room for its label).
    pub header: f32,
    /// Padding around a nested directory's children.
    pub pad: f32,
    /// Tiles thinner than this are dropped (the parent's fill shows instead).
    pub min_side: f32,
    /// A directory nests its children only if it is at least this big on both sides.
    pub min_nest: f32,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions { max_depth: 12, header: 16., pad: 2., min_side: 1.5, min_nest: 24. }
    }
}

/// Nested tiles for the subtree under `root`, parents before children (paint in order).
pub fn layout(tree: &Tree, root: NodeId, r: Rect, o: &LayoutOptions) -> Vec<Tile> {
    let mut out = Vec::new();
    place(tree, root, r, 0, o, &mut out);
    out
}

fn place(tree: &Tree, parent: NodeId, r: Rect, depth: u16, o: &LayoutOptions, out: &mut Vec<Tile>) {
    let kids: Vec<NodeId> = tree.node(parent).children.iter().copied().filter(|&c| tree.node(c).size > 0).collect();
    if kids.is_empty() {
        return;
    }
    let weights: Vec<f64> = kids.iter().map(|&c| tree.node(c).size as f64).collect();
    // scale to the parent's total so its own directory-entry bytes leave a little unused space
    let total = tree.node(parent).size.max(1) as f64;
    let used: f64 = weights.iter().sum();
    let frac = (used / total).clamp(0., 1.) as f32;
    let r = if r.w >= r.h { Rect { w: r.w * frac, ..r } } else { Rect { h: r.h * frac, ..r } };
    for (&id, rect) in kids.iter().zip(squarify(&weights, r)) {
        if rect.w < o.min_side || rect.h < o.min_side {
            continue;
        }
        let node = tree.node(id);
        let nested = node.is_dir() && !node.children.is_empty() && depth < o.max_depth && rect.w >= o.min_nest && rect.h >= o.min_nest + o.header;
        out.push(Tile { node: id, rect, depth, nested });
        if nested {
            let inner = Rect::new(rect.x + o.pad, rect.y + o.header, rect.w - 2. * o.pad, rect.h - o.header - o.pad);
            place(tree, id, inner, depth + 1, o, out);
        }
    }
}

/// Deepest tile containing the point, if any.
pub fn hit(tiles: &[Tile], x: f32, y: f32) -> Option<&Tile> {
    tiles.iter().rev().find(|t| t.rect.contains(x, y))
}
