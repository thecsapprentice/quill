//! Quill's desktop app: quill-ui's views over the native backend, in a
//! Dioxus desktop window.

use std::{rc::Rc, sync::Arc};

use dioxus::prelude::*;
use quill_core::store::vault::{OsKeychain, Vault};
use quill_gui::{NativeBackend, settings::Paths};
use quill_ui::{App, STYLE, Ui};

fn main() {
    let (dom, config, _runtime) = match setup() {
        Ok(parts) => parts,
        Err(e) => {
            eprintln!("quill-gui: {e:#}");
            std::process::exit(1);
        }
    };
    // Runs until the window closes and never returns; `_runtime` lives
    // until then.
    dioxus_desktop::launch::launch_virtual_dom(dom, config);
}

/// The window's root and settings, and the runtime quill-core runs on.
fn setup() -> anyhow::Result<(VirtualDom, dioxus_desktop::Config, tokio::runtime::Runtime)> {
    // quill-core's work runs here, off the window's thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("quill-core")
        .enable_all()
        .build()?;
    // No keychain (some Linux setups): stores take a passphrase instead.
    let vault = OsKeychain::new()
        .ok()
        .map(|k| Arc::new(k) as Arc<dyn Vault>);
    let backend = NativeBackend::new(Paths::default_paths()?, vault, runtime.handle().clone())?;

    let dom = VirtualDom::new(Root);
    dom.provide_root_context(Ui(Rc::new(backend)));
    let config = dioxus_desktop::Config::new()
        .with_menu(None)
        .with_custom_head(format!("<style>{STYLE}</style>"))
        .with_background_color((238, 241, 244, 255))
        .with_window(
            dioxus_desktop::WindowBuilder::new()
                .with_title("Quill")
                .with_inner_size(dioxus_desktop::LogicalSize::new(1180.0, 780.0))
                .with_min_inner_size(dioxus_desktop::LogicalSize::new(360.0, 480.0)),
        );
    Ok((dom, config, runtime))
}

/// The platform's part of the root: spellcheck, then quill-ui's app.
#[component]
fn Root() -> Element {
    use_hook(spellcheck);
    rsx! { App {} }
}

/// WebKitGTK checks spelling only when asked, unlike WebView2 and
/// WKWebView (3a's findings).
#[cfg(target_os = "linux")]
fn spellcheck() {
    use dioxus_desktop::wry::WebViewExtUnix;
    use webkit2gtk::{WebContextExt, WebViewExt};
    if let Some(context) = dioxus_desktop::window().webview.webview().context() {
        context.set_spell_checking_enabled(true);
        let lang = std::env::var("LANG").unwrap_or_default();
        let lang = lang
            .split('.')
            .next()
            .filter(|l| !l.is_empty() && *l != "C" && *l != "POSIX")
            .unwrap_or("en_US");
        context.set_spell_checking_languages(&[lang]);
    }
}

#[cfg(not(target_os = "linux"))]
fn spellcheck() {}
