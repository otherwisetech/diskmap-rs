//! The treemap: tiles painted as quads in one `canvas`, labels as absolutely positioned divs for
//! the biggest tiles only, hover/selection outlines, and a status line under it.
use crate::app::DiskApp;
use crate::theme::{Tokens, tokens};
use gpui::*;
use scan::treemap::{LayoutOptions, Rect, hit, layout};
use scan::{Tree, human};
use std::rc::Rc;

const MAX_LABELS: usize = 260;
const HEADER: f32 = 18.;

impl DiskApp {
    fn ensure_tiles(&mut self) {
        let Some(tree) = &self.tree else { return };
        let b = self.canvas_bounds.get();
        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
        let key = (self.rev, self.zoom, w as i32, h as i32);
        if self.tiles_key == Some(key) {
            return;
        }
        self.tiles_key = Some(key);
        let opts = LayoutOptions { header: HEADER, pad: 3., ..LayoutOptions::default() };
        self.tiles = Rc::new(if w > 0. && h > 0. { layout(tree, self.zoom, Rect::new(0., 0., w, h), &opts) } else { Vec::new() });
    }

    pub(crate) fn render_treemap(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx);
        self.ensure_tiles();
        let hits = self.search_hits();
        let Some(tree) = &self.tree else { return div().into_any_element() };
        let tiles = self.tiles.clone();

        let mut quads: Vec<(Rect, Hsla)> = Vec::with_capacity(tiles.len());
        for tile in tiles.iter() {
            let node = tree.node(tile.node);
            let (r, color) = if node.is_dir() {
                (tile.rect, if tile.nested { t.dir(tile.depth) } else { t.dir(tile.depth + 2) })
            } else {
                (inset(tile.rect, 0.5), t.kind(node.kind))
            };
            quads.push((r, color));
        }
        let rect_of = |id| tiles.iter().find(|x| x.node == id).map(|x| x.rect);
        let hover_rect = self.hovered.and_then(rect_of);
        let sel_rect = self.selected.and_then(rect_of);
        let hit_set: std::collections::HashSet<_> = hits.iter().copied().collect();
        let hit_rects: Vec<Rect> = if hit_set.is_empty() { Vec::new() } else { tiles.iter().filter(|x| hit_set.contains(&x.node)).map(|x| x.rect).collect() };

        let bounds_cell = self.canvas_bounds.clone();
        let (hover_c, sel_c, frame, hit_c) = (t.ink.opacity(0.85), t.accent, t.canvas, t.accent.opacity(0.8));
        let paint = canvas(
            move |b, window, _| {
                if bounds_cell.get() != b {
                    bounds_cell.set(b);
                    window.refresh();
                }
            },
            move |b, _, window, _| {
                window.paint_quad(fill(b, frame));
                for (r, c) in &quads {
                    window.paint_quad(fill(to_bounds(b, *r), *c));
                }
                for r in &hit_rects {
                    window.paint_quad(quad(to_bounds(b, inset(*r, 0.5)), px(1.), transparent_black(), px(1.5), hit_c, BorderStyle::Dashed));
                }
                if let Some(r) = hover_rect {
                    window.paint_quad(quad(to_bounds(b, r), px(0.), transparent_black(), px(1.5), hover_c, BorderStyle::Solid));
                }
                if let Some(r) = sel_rect {
                    window.paint_quad(quad(to_bounds(b, inset(r, -1.)), px(2.), transparent_black(), px(2.5), sel_c, BorderStyle::Solid));
                }
            },
        )
        .absolute()
        .inset_0();

        // labels for the biggest tiles that have room for them
        let mut cand: Vec<usize> = (0..tiles.len()).filter(|&i| {
            let tl = &tiles[i];
            if tl.nested { tl.rect.w >= 56. } else { tl.rect.w >= 70. && tl.rect.h >= 34. }
        }).collect();
        cand.sort_by(|a, b| tiles[*b].rect.area().total_cmp(&tiles[*a].rect.area()));
        cand.truncate(MAX_LABELS);
        let labels: Vec<AnyElement> = cand.into_iter().map(|i| label(&t, tree, &tiles[i])).collect();

        let status = self.render_status(&t);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("treemap")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .m(px(8.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(paint)
                    .children(labels)
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        let h = this.hit(e.position);
                        if h != this.hovered {
                            this.hovered = h;
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(|this, inside: &bool, _, cx| {
                        if !*inside && this.hovered.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            let h = this.hit(e.position);
                            match (h, e.click_count) {
                                (Some(id), n) if n >= 2 => this.zoom_into(id, cx),
                                (h, _) => this.select(h, cx),
                            }
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            if let Some(id) = this.hit(e.position) {
                                this.select_and_copy(id, cx);
                            }
                        }),
                    ),
            )
            .child(status)
            .into_any_element()
    }

    fn hit(&self, pos: Point<Pixels>) -> Option<scan::NodeId> {
        let b = self.canvas_bounds.get();
        if !b.contains(&pos) {
            return None;
        }
        let (x, y) = (f32::from(pos.x - b.origin.x), f32::from(pos.y - b.origin.y));
        hit(&self.tiles, x, y).map(|t| t.node)
    }

    fn render_status(&self, t: &Tokens) -> impl IntoElement {
        let text = match (&self.tree, self.hovered) {
            (Some(tree), Some(id)) => {
                let n = tree.node(id);
                let pct = pct(n.size, tree.node(self.zoom).size);
                format!("{}  ·  {}  ·  {}", tree.path(id).display(), human(n.size), i18n::t!("app.panel.share", pct = pct))
            }
            _ => i18n::t!("app.status.hint", copy = crate::app::COPY_KEYS),
        };
        div().flex_none().h(px(26.)).px(px(12.)).flex().items_center().text_size(px(12.)).text_color(t.ink_muted).truncate().child(text)
    }
}

fn label(t: &Tokens, tree: &Tree, tile: &scan::treemap::Tile) -> AnyElement {
    let n = tree.node(tile.node);
    let r = tile.rect;
    let (fg, muted) = if n.is_dir() { (t.ink, t.ink_muted) } else { (t.on_kind(), t.on_kind().opacity(0.8)) };
    let base = div().absolute().left(px(r.x + 4.)).top(px(r.y + 2.)).w(px((r.w - 8.).max(0.))).overflow_hidden().text_size(px(11.)).line_height(px(14.));
    if tile.nested {
        base.h(px(HEADER - 2.)).flex().gap(px(6.)).child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).text_color(fg).child(n.name.to_string())).child(div().flex_none().text_color(muted).child(human(n.size))).into_any_element()
    } else {
        base.h(px((r.h - 4.).max(0.))).flex().flex_col().child(div().truncate().font_weight(FontWeight::MEDIUM).text_color(fg).child(n.name.to_string())).child(div().truncate().text_color(muted).child(human(n.size))).into_any_element()
    }
}

fn inset(r: Rect, d: f32) -> Rect {
    if r.w <= 2. * d || r.h <= 2. * d {
        return r;
    }
    Rect::new(r.x + d, r.y + d, r.w - 2. * d, r.h - 2. * d)
}

fn to_bounds(b: Bounds<Pixels>, r: Rect) -> Bounds<Pixels> {
    Bounds { origin: point(b.origin.x + px(r.x), b.origin.y + px(r.y)), size: size(px(r.w), px(r.h)) }
}

pub fn pct(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0".into();
    }
    let p = part as f64 * 100. / whole as f64;
    if p >= 10. || p == 0. { format!("{p:.0}") } else { format!("{p:.1}") }
}

