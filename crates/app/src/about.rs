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
        let task = cx.background_executor().spawn(async { crate::otherwise::other_apps(&otherwise_link::apps::Locations::system()) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| {
                this.other_apps = found;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "More Otherwise apps": the ones installed here, each opening on click.
    fn render_other_apps(&self, t: &Tokens) -> Option<Div> {
        if self.other_apps.is_empty() {
            return None;
        }
        let rows = self.other_apps.iter().enumerate().map(|(i, (spec, installed))| {
            let installed = installed.clone();
            let hover = t.canvas;
            let version = installed.version.clone().unwrap_or_default();
            div()
                .id(("about-app", i))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(div().text_size(px(13.)).text_color(t.ink).child(spec.name.clone()))
                .child(div().flex().items_center().gap(px(6.)).text_size(px(11.)).text_color(t.ink_muted).child(version).child(icon(IconName::ExternalLink, 12., t.ink_muted)))
                .on_click(move |_, _, _| {
                    let app = installed.clone();
                    std::thread::spawn(move || crate::otherwise::launch(&app));
                })
        });
        Some(div().mt(px(18.)).w_full().flex().flex_col().gap(px(4.)).child(ui::section_title(t, i18n::t!("app.about.more_apps"))).children(rows))
    }

    pub fn close_about(&mut self, cx: &mut Context<Self>) {
        if self.about_open {
            self.about_open = false;
            cx.notify();
        }
    }

    /// The family's footer: `status` on the left, the star link, then "diskmap 0.6.2 ·
    /// Otherwise Technologies LLC · Acerca de", opening the About modal.
    pub(crate) fn render_footer(&self, t: &Tokens, status: impl IntoElement, cx: &mut Context<Self>) -> otherwise_ui::AppFooter {
        otherwise_ui::AppFooter::new(APP_NAME, i18n::t!("app.about.footer_link")).version(VERSION).status(status).item(crate::star::footer_link(t, cx)).on_about(cx.listener(|this, _, window, cx| this.show_about(window, cx)))
    }

    pub(crate) fn render_about(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let links = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(otherwise_ui::link(t, "about-source").flex().items_center().gap(px(6.)).text_size(px(otherwise_ui::tokens::text::SMALL)).child(icon(IconName::Github, 14., t.accent)).child(i18n::t!("app.about.source")).on_click(|_, _, cx| cx.open_url(REPO_URL)))
            .child(crate::star::about_button(t, cx));
        let mut about = otherwise_ui::AboutModal::new(APP_NAME, VERSION, i18n::t!("app.about.by", company = COMPANY))
            .logo(self.logo.clone())
            .description(i18n::t!("app.about.description"))
            .place(i18n::t!("app.about.location"))
            .website(i18n::t!("app.about.website"), COMPANY_URL)
            .vcity(i18n::t!("app.vcity.pitch"), crate::vcity::menu_label(), crate::vcity::url(crate::otherwise::APP_ID))
            .extra(links)
            .extra(crate::star::about_ask(t));
        if let Some(apps) = self.render_other_apps(t) {
            about = about.extra(apps);
        }
        about.license(i18n::t!("app.about.license")).on_dismiss(cx.listener(|this, _: &otherwise_ui::Dismiss, _, cx| this.close_about(cx)))
    }
}
