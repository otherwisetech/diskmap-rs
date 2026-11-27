//! Small styled building blocks shared by the top bar, panel and screens.
use crate::theme::Tokens;
use gpui::*;
use gpui_component::IconName;
pub use otherwise_ui::{filled_button, icon, outline_button, section_title};

/// The family's square icon-only button with one of gpui-component's icons.
pub fn icon_button(t: &Tokens, id: impl Into<ElementId>, name: IconName, enabled: bool) -> Stateful<Div> {
    otherwise_ui::icon_button(t, id, enabled).child(icon(name, otherwise_ui::tokens::icon::TOOLBAR, if enabled { t.ink } else { t.ink_faint }))
}

pub fn swatch(color: Hsla, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded(px(3.)).bg(color)
}
