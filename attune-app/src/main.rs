//! Attune's window.
//!
//! The window itself lives in [`window`] and is compiled only on Windows,
//! because it is built on WebView2 -- and because `tao` and `wry` drag GTK3
//! and WebKitGTK into the dependency graph on Linux, which upstream's CI has
//! no reason to install for a product that ships to Windows 11 only. Gating
//! the crates here rather than adding `libgtk-3-dev` to upstream's workflow
//! keeps `.github/workflows/build.yml` untouched, which is the whole reason
//! `attune-ci.yml` exists as a separate file.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod window;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    window::run()
}

/// Present so the crate still has a `main` on platforms it does not target --
/// a binary crate without one does not compile, and `cargo check --workspace`
/// on Linux is exactly what caught this.
#[cfg(not(windows))]
fn main() {
    eprintln!("attune-app is the Windows desktop window for the Attune daemon.");
    eprintln!("This platform is not supported; run the daemon and open its UI directly.");
    std::process::exit(1);
}
