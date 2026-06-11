// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Force GTK to use the Wayland backend — without this, GTK defaults to
    // XWayland and gtk-layer-shell has no Wayland display to attach to.
    // WEBKIT_DISABLE_DMABUF_RENDERER prevents a WebKit crash on KDE Wayland.
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("GDK_BACKEND", "wayland");
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    exilewatch_lib::run()
}
