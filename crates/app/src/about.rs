//! About modal and the footer line that opens it.
use crate::app::DiskApp;
use crate::theme::Tokens;
use crate::ui::{self, icon};
use crate::{APP_NAME, COMPANY, COMPANY_URL, REPO_URL, VERSION};
use gpui::*;
use gpui_component::IconName;

impl DiskApp {
    pub fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.about_open = true;
        // keys must not keep typing into the search field behind the modal
        self.focus_main(window, cx);
        cx.notify();
    }

    pub fn close_about(&mut self, cx: &mut Context<Self>) {
        if self.about_open {
            self.about_open = false;
            cx.notify();
        }
    }

    /// "diskmap 0.1.0 · Otherwise Technologies LLC · About", opening the About modal.
    pub(crate) fn render_footer(&self, t: &Tokens, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let ink = t.ink;
        div()
            .id(id)
            .flex_none()
            .text_size(px(12.))
            .text_color(t.ink_muted)
            .cursor_pointer()
            .hover(move |s| s.text_color(ink).underline())
            .child(i18n::t!("app.about.footer", app = APP_NAME, version = VERSION, company = COMPANY))
            .on_click(cx.listener(|this, _, window, cx| this.show_about(window, cx)))
    }

    pub(crate) fn render_about(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = t.accent;
        let source = div()
            .id("about-source")
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .text_color(t.accent)
            .cursor_pointer()
            .hover(move |s| s.text_color(accent.opacity(0.8)).underline())
            .child(icon(IconName::Github, 14., t.accent))
            .child(i18n::t!("app.about.source"))
            .on_click(|_, _, cx| cx.open_url(REPO_URL));
        let card = div()
            .id("about")
            .relative()
            .w(px(380.))
            .flex()
            .flex_col()
            .items_center()
            .p(px(28.))
            .pt(px(32.))
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().absolute().top(px(10.)).right(px(10.)).child(ui::icon_button(t, "about-close", IconName::Close, true).on_click(cx.listener(|this, _, _, cx| this.close_about(cx)))))
            .child(img(self.logo.clone()).size(px(88.)))
            .child(div().mt(px(14.)).text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(APP_NAME))
            .child(div().mt(px(2.)).text_size(px(12.)).text_color(t.ink_muted).child(i18n::t!("app.about.version", version = VERSION)))
            .child(div().mt(px(14.)).text_center().text_size(px(13.)).line_height(px(19.)).text_color(t.ink).child(i18n::t!("app.about.description")))
            .child(
                div()
                    .mt(px(18.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .text_size(px(12.))
                    .child(div().text_color(t.ink).child(i18n::t!("app.about.by", company = COMPANY)))
                    .child(div().text_color(t.ink_muted).child(i18n::t!("app.about.location"))),
            )
            .child(ui::filled_button(t, "about-site", t.accent).mt(px(20.)).child(icon(IconName::ExternalLink, 14., t.on_fill())).child(i18n::t!("app.about.website")).on_click(|_, _, cx| cx.open_url(COMPANY_URL)))
            .child(div().mt(px(12.)).child(source))
            .child(div().mt(px(16.)).text_size(px(11.)).text_color(t.ink_muted).child(i18n::t!("app.about.license")));
        div()
            .id("about-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(black().opacity(if t.dark { 0.55 } else { 0.32 }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_about(cx)))
            .child(card)
    }
}
