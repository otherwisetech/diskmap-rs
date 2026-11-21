// No console window behind the app on Windows release builds.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod about;
mod app;
mod dev;
#[cfg(windows)]
mod explorer;
mod inbox;
mod otherwise;
mod panel;
#[cfg(target_os = "macos")]
mod services;
mod settings;
mod storage;
mod vcity;
mod star;
mod theme;
mod treemap;
mod ui;

use gpui::*;
use gpui_component::Root;
use gpui_component_assets::Assets;
use std::borrow::Cow;
use std::path::PathBuf;

actions!(diskmap, [Quit, OpenWebsite, ShowAbout, OpenVcity, RevealData]);

pub(crate) const APP_NAME: &str = "diskmap"; // i18n-ok: product name
pub(crate) const COMPANY: &str = "Otherwise Technologies LLC"; // i18n-ok: company name
pub(crate) const COMPANY_URL: &str = "https://otherwisetech.com";
pub(crate) const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The native menu bar holds plain strings, so it is rebuilt on every language switch.
pub(crate) fn set_app_menus(cx: &mut App) {
    let items = vec![
        MenuItem::action(i18n::t!("app.about.menu", app = APP_NAME), ShowAbout),
        MenuItem::separator(),
        MenuItem::action(i18n::t!("app.credit.menu", company = COMPANY), OpenWebsite),
        MenuItem::separator(),
        MenuItem::action(i18n::t!("app.menu.reveal_data"), RevealData),
        MenuItem::separator(),
        MenuItem::action(i18n::t!("app.menu.quit"), Quit),
    ];
    let help = vec![MenuItem::action(vcity::menu_label(), OpenVcity)];
    cx.set_menus(vec![Menu { name: APP_NAME.into(), items, disabled: false }, Menu { name: i18n::t!("app.menu.help").into(), items: help, disabled: false }]);
}

/// Handled at app level so the menu item works wherever focus is.
fn show_about(_: &ShowAbout, cx: &mut App) {
    for handle in cx.windows() {
        let Some(root) = handle.downcast::<Root>() else { continue };
        root.update(cx, |root, window, cx| {
            if let Ok(view) = root.view().clone().downcast::<app::DiskApp>() {
                view.update(cx, |this, cx| this.show_about(window, cx));
            }
        })
        .ok();
    }
}

/// gpui-component's icon set plus our own `icons/*.svg`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            LOGO_SVG => Ok(Some(Cow::Borrowed(include_bytes!("../assets/icons/diskmap-logo.svg")))),
            _ => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Assets.list(path)
    }
}

pub(crate) const LOGO_SVG: &str = "icons/diskmap-logo.svg";
pub(crate) static ICON_PNG: &[u8] = include_bytes!("../assets/logo/diskmap-icon-512.png");

/// Sets the Dock icon at runtime: covers `cargo run` (no bundle) and stale Dock icon caches.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc 0.2 macros check a `cargo-clippy` feature
fn set_dock_icon() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    static PNG: &[u8] = include_bytes!("../assets/logo/diskmap-icon-1024.png");
    // SAFETY: plain AppKit calls on the main thread with a valid byte buffer that NSData copies.
    unsafe {
        let data: *mut Object = msg_send![class!(NSData), dataWithBytes: PNG.as_ptr() length: PNG.len()];
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: data];
        if image.is_null() {
            return;
        }
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
        let _: () = msg_send![image, release];
    }
}

const FONTS: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
];

fn scan_path(a: otherwise::Action) -> Option<PathBuf> {
    match a {
        otherwise::Action::Scan(p) => Some(p),
        otherwise::Action::Focus | otherwise::Action::Dev => None,
    }
}

fn main() {
    storage::migrate_on_startup();
    i18n::set_locale(settings::initial_language(&settings::Settings::load()));
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("{}", i18n::t!("app.cli.usage"));
        return;
    }
    let initial = arg.as_deref().map(inbox::arg_action);
    otherwise::register_scheme();
    otherwise::watch_today();
    let application = gpui_platform::application().with_assets(AppAssets);
    application.on_open_urls(inbox::send_urls);
    application.run(move |cx| {
        if let Err(e) = cx.text_system().add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
            eprintln!("fonts: {e}");
        }
        #[cfg(target_os = "macos")]
        {
            set_dock_icon();
            services::register();
        }
        gpui_component::init(cx);
        app::sync_theme(None, cx);
        app::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenWebsite, cx| cx.open_url(COMPANY_URL));
        cx.on_action(|_: &OpenVcity, cx| vcity::open(cx));
        cx.on_action(|_: &RevealData, cx| storage::reveal_data_dir(cx));
        cx.on_action(show_about);
        gpui_component::set_locale(i18n::locale());
        set_app_menus(cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some(APP_NAME.into()), ..Default::default() }),
            ..Default::default()
        };
        let inbox = inbox::receiver();
        // A folder that arrived during launch (Finder service, `open -a diskmap <dir>`) wins over argv.
        let launched: Vec<otherwise::Action> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        let want_dev = launched.contains(&otherwise::Action::Dev) || initial == Some(otherwise::Action::Dev);
        let initial = launched.into_iter().filter_map(scan_path).next_back().or_else(|| initial.clone().and_then(scan_path));
        cx.spawn(async move |cx| {
            let handle = cx
                .open_window(window_options, |window, cx| {
                    let view = cx.new(|cx| {
                        let mut this = app::DiskApp::new(initial, window, cx);
                        if want_dev {
                            this.open_dev(window, cx);
                        }
                        this
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("open window");
            while let Ok(action) = inbox.recv().await {
                cx.update(|cx| cx.activate(true));
                handle
                    .update(cx, |root, window, cx| {
                        window.activate_window();
                        let Ok(view) = root.view().clone().downcast::<app::DiskApp>() else { return };
                        if action == otherwise::Action::Dev {
                            view.update(cx, |this, cx| this.open_dev(window, cx));
                        } else if let Some(path) = scan_path(action) {
                            view.update(cx, |this, cx| this.start_scan(path, window, cx));
                        }
                    })
                    .ok();
            }
        })
        .detach();
    });
}
