//! Right panel: what is selected (or the folder in view), actions, contents, biggest files, legend.
use crate::app::DiskApp;
use crate::theme::Tokens;
use crate::treemap::pct;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::IconName;
use scan::{Kind, NodeId, Tree, human};

const PANEL_W: f32 = 340.;
const ROW_H: f32 = 30.;

fn kind_name(k: Kind) -> String {
    i18n::t!(&format!("app.kind.{}", k.id()))
}

impl DiskApp {
    pub(crate) fn render_panel(&mut self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let largest = self.largest_files();
        let Some(tree) = &self.tree else { return div().into_any_element() };
        let id = self.target();
        let n = tree.node(id);
        let zoom_size = tree.node(self.zoom).size;
        let root_size = tree.node(Tree::ROOT).size;
        let is_root = id == Tree::ROOT;
        let can_enter = n.is_dir() && !n.children.is_empty() && id != self.zoom;

        let mut facts = div().flex().flex_col().gap(px(3.)).text_size(px(12.)).text_color(t.ink_muted);
        if n.is_dir() {
            facts = facts.child(i18n::tn!("app.panel.files", n.files));
        }
        if id != self.zoom {
            facts = facts.child(i18n::t!("app.panel.share", pct = pct(n.size, zoom_size)));
        }
        if !is_root {
            facts = facts.child(i18n::t!("app.panel.share_root", pct = pct(n.size, root_size)));
        }
        let unreadable = n.unreadable || (is_root && self.scan_errors > 0);
        let errors = self.scan_errors;

        let header = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(if n.is_dir() { icon(IconName::Folder, 16., t.ink_muted).into_any_element() } else { ui::swatch(t.kind(n.kind), 12.).into_any_element() })
                    .child(div().min_w_0().flex_1().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(n.name.to_string())),
            )
            .child(div().flex().items_baseline().gap(px(8.)).child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).child(human(n.size))).child(div().text_size(px(12.)).text_color(t.ink_muted).child(kind_name(n.kind))))
            .child(facts)
            .child({
                let ink = t.ink;
                div()
                    .id("path")
                    .text_size(px(11.))
                    .text_color(t.ink_muted)
                    .truncate()
                    .cursor_pointer()
                    .hover(move |s| s.text_color(ink).underline())
                    .child(tree.path(id).display().to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_path(id, cx)))
            })
            .when(unreadable, |d| {
                d.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(t.warning)
                        .child(icon(IconName::TriangleAlert, 14., t.warning).flex_none())
                        .child(div().flex_1().child(if is_root { i18n::tn!("app.panel.unreadable_some", errors) } else { i18n::t!("app.panel.unreadable") })),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .when(can_enter, |d| d.child(ui::outline_button(t, "enter").child(icon(IconName::ArrowRight, 14., t.ink)).child(i18n::t!("app.panel.enter")).on_click(cx.listener(move |this, _, _, cx| this.zoom_into(id, cx)))))
                    .child(ui::outline_button(t, "reveal").child(icon(IconName::ExternalLink, 14., t.ink)).child(i18n::t!("app.panel.reveal")).on_click(cx.listener(move |this, _, _, cx| this.reveal(id, cx))))
                    .child(ui::outline_button(t, "copy").child(icon(IconName::Copy, 14., t.ink)).child(i18n::t!("app.panel.copy")).on_click(cx.listener(move |this, _, _, cx| this.copy_path(id, cx))))
                    .when(!is_root, |d| {
                        let danger = t.danger;
                        d.child(ui::outline_button(t, "trash").text_color(danger).child(icon(IconName::Delete, 14., danger)).child(i18n::t!("app.panel.trash")).on_click(cx.listener(move |this, _, window, cx| this.move_to_trash(id, window, cx))))
                    }),
            );

        let zoom = self.zoom;
        let count = tree.node(zoom).children.len();
        let selected = self.selected;
        let contents = uniform_list(
            "contents",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = crate::theme::tokens(cx);
                let Some(tree) = &this.tree else { return Vec::new() };
                let parent = tree.node(zoom);
                let max = parent.children.first().map(|&c| tree.node(c).size).unwrap_or(1).max(1);
                range.filter_map(|ix| parent.children.get(ix).map(|&c| row(&t, tree, c, max, selected == Some(c), ix, cx))).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list_scroll)
        .flex_1()
        .min_h(px(120.));

        let big: Vec<AnyElement> = largest.iter().enumerate().map(|(ix, &c)| row(t, tree, c, largest.first().map(|&f| tree.node(f).size).unwrap_or(1).max(1), selected == Some(c), 10_000 + ix, cx)).collect();

        let legend = div().flex().flex_wrap().gap_x(px(12.)).gap_y(px(6.)).p(px(16.)).border_t_1().border_color(t.border).children(Kind::LEGEND.iter().map(|&k| div().flex().items_center().gap(px(5.)).text_size(px(11.)).text_color(t.ink_muted).child(ui::swatch(t.kind(k), 9.)).child(kind_name(k))));

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(PANEL_W))
            .h_full()
            .bg(t.surface)
            .border_l_1()
            .border_color(t.border)
            .child(header)
            .child(div().px(px(16.)).pt(px(14.)).pb(px(6.)).child(ui::section_title(t, i18n::t!("app.panel.contents", name = tree.node(zoom).name))))
            .child(contents)
            .when(!big.is_empty(), |d| d.child(div().px(px(16.)).pt(px(12.)).pb(px(6.)).border_t_1().border_color(t.border).child(ui::section_title(t, i18n::t!("app.panel.largest")))).child(div().flex().flex_col().pb(px(6.)).children(big)))
            .child(legend)
            .into_any_element()
    }
}

fn row(t: &Tokens, tree: &Tree, id: NodeId, max: u64, selected: bool, ix: usize, cx: &mut Context<DiskApp>) -> AnyElement {
    let n = tree.node(id);
    let color = if n.is_dir() { t.ink_muted.opacity(0.55) } else { t.kind(n.kind) };
    let frac = (n.size as f64 / max as f64).clamp(0., 1.) as f32;
    let hover = t.canvas;
    div()
        .id(("row", ix))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(ROW_H))
        .px(px(16.))
        .cursor_pointer()
        .when(selected, |d| d.bg(t.accent_soft))
        .hover(move |s| s.bg(hover))
        .child(if n.is_dir() { icon(IconName::Folder, 13., t.ink_muted).into_any_element() } else { ui::swatch(color, 9.).into_any_element() })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(div().truncate().text_size(px(12.)).child(n.name.to_string()))
                .child(div().h(px(3.)).w_full().rounded_full().bg(t.border).child(div().h_full().rounded_full().bg(color).w(relative(frac)))),
        )
        .child(div().flex_none().w(px(64.)).text_right().text_size(px(12.)).text_color(t.ink_muted).child(human(n.size)))
        .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| if e.click_count() >= 2 { this.zoom_into(id, cx) } else { this.select(Some(id), cx) }))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| this.select_and_copy(id, cx)))
        .into_any_element()
}
