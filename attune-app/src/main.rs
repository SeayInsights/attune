//! Attune's window.
//!
//! The window itself lives in [`window`] and is compiled only on Windows,
//! because it is built on WebView2 -- and because `tao` and `wry` drag GTK3
//! and WebKitGTK into the dependency graph on Linux, which upstream's CI has
//! no reason to install for a product that ships to Windows 11 only.
//!
//! Gating beat adding `libgtk-3-dev` to upstream's workflow because it was
//! cheap: two crates and a stub `main`, against a system library installed
//! for a platform nothing ships to. The same call went the other way for
//! ALSA in the same change -- `attune-analysis` takes `cpal` unconditionally
//! and gating it would have cascaded through capture, meters, playback and
//! every call site, so `build.yml` gained `libasound2-dev` instead. The
//! principle is the cost of the gate, not an unbroken rule about never
//! touching inherited files.

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
