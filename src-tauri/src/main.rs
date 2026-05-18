// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
fn configure_macos_menubar_app() {
    use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyAccessory};

    unsafe {
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    configure_macos_menubar_app();

    app_lib::run();
}
