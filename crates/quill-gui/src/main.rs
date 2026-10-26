//! Quill's desktop app.

use dioxus::prelude::*;

fn main() {
    let config = dioxus_desktop::Config::new().with_menu(None).with_window(
        dioxus_desktop::WindowBuilder::new()
            .with_title("Quill")
            .with_inner_size(dioxus_desktop::LogicalSize::new(1180.0, 780.0)),
    );
    dioxus_desktop::launch::launch_virtual_dom(VirtualDom::new(app), config);
}

fn app() -> Element {
    rsx! { "Quill" }
}
