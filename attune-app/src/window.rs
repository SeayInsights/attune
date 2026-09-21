//! Attune's window.
//!
//! # Why this exists
//!
//! Upstream's UI is a web page the daemon serves, and upstream opens it by
//! shell-opening the URL -- so the application arrives as a tab in whatever
//! browser happens to be default, sharing a window with everything else, with
//! an address bar and a set of browser keyboard shortcuts that have nothing to
//! do with a mixer. Installed software should open as itself.
//!
//! So this is a window. It hosts the same page through WebView2, which is the
//! engine Edge already uses and which ships with Windows -- there is no second
//! browser here, no bundled Chromium, and the UI is byte for byte the one the
//! daemon serves. What changes is the frame around it.
//!
//! # What it does not do
//!
//! Reimplement the daemon, or talk to the GoXLR. It starts the daemon if it is
//! not already running, waits for it to answer, and shows its page. Everything
//! else is unchanged, which is the point: upstream's daemon and web UI stay
//! exactly as they are and remain mergeable.

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::{WebContext, WebViewBuilder};

/// Where the daemon serves its UI. Upstream's fixed port.
const PORT: u16 = 14564;

/// How long to wait for a daemon that was just started.
///
/// Generous: it enumerates USB devices and loads profiles before it listens,
/// and the first run after an install is the slowest. Giving up early would
/// show an error on exactly the run most likely to be someone's first.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

pub fn run() -> Result<()> {
    if let Err(error) = ensure_daemon_running() {
        // A message box rather than a log line: this is a windowed binary, so
        // there is no console for anything written to stderr to land in.
        report(&format!(
            "Attune could not start its background service.\n\n{error}"
        ));
        return Err(error);
    }

    // Best effort, and deliberately not fatal: the window opens either way.
    register_as_activator();

    let event_loop = EventLoopBuilder::new().build();
    let window = WindowBuilder::new()
        .with_title("Attune")
        .with_window_icon(window_icon())
        // Wide enough for the mixer's faders and the equaliser plot side by
        // side, which is what the layout is designed around.
        .with_inner_size(LogicalSize::new(1280.0, 860.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .build(&event_loop)
        .context("could not create the window")?;

    // WebView2 keeps a profile -- cache, storage, logs -- and by default puts
    // it next to the executable. For an installed copy that is Program Files,
    // which is not writable without administrator rights, so the default is
    // wrong twice over: it would fail where it should not, and it leaves a
    // folder behind in the install directory that the uninstaller knows
    // nothing about. Measured: a test install left 200-odd files there.
    let mut context = WebContext::new(webview_data_directory());

    let _webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(format!("http://127.0.0.1:{PORT}"))
        // The page is local and there is no address bar, so a context menu
        // offering "view source" and "back" is only ever a way to get lost.
        .with_devtools(cfg!(debug_assertions))
        .build(&window)
        .context(
            "could not create the web view. Attune uses the WebView2 runtime, \
             which ships with Windows 11 and can be installed on Windows 10 \
             from Microsoft's Edge WebView2 page.",
        )?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            // Closing the window leaves the daemon running, deliberately. It
            // holds the device's state and the tray icon, and people expect
            // their mic to keep working after they close a mixer window.
            *control_flow = ControlFlow::Exit;
        }
    })
}

/// The icon shown in the title bar, the taskbar and Alt-Tab.
///
/// Separate from the one the resource script embeds. That gives the *file* an
/// icon, which is what Explorer and the Start menu shortcut use; a window's
/// icon is a different thing entirely, and Windows draws a placeholder until
/// something sets it.
///
/// A PNG rather than the .ico the resource script uses, deliberately. The .ico
/// stores its frames as PNGs that are not in RGBA, and the ICO decoder rejects
/// exactly that with `PngNotRgba` -- so reading the container fails while the
/// frame inside it is perfectly good. Shipping that frame directly sidesteps a
/// container that is only there for Explorer's benefit.
///
/// Returns `None` rather than failing if it cannot be decoded. A missing icon
/// is a blemish, and refusing to open the application over one would be out of
/// all proportion to it -- which is exactly why there is a test: a silent
/// fallback is only safe if something else is watching.
fn window_icon() -> Option<Icon> {
    const ICON: &[u8] = include_bytes!("../resources/attune.png");

    let image = image::load_from_memory_with_format(ICON, image::ImageFormat::Png)
        .inspect_err(|e| log::warn!("could not decode the window icon: {e}"))
        .ok()?
        .into_rgba8();

    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .inspect_err(|e| log::warn!("could not build the window icon: {e}"))
        .ok()
}

/// Where WebView2 keeps its profile: under the user's local app data,
/// rather than wherever the application happens to be installed.
fn webview_data_directory() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var("LOCALAPPDATA").ok()?);
    let directory = base.join("Attune").join("WebView2");
    std::fs::create_dir_all(&directory)
        .inspect_err(|e| log::warn!("could not create {}: {e}", directory.display()))
        .ok()?;
    Some(directory)
}

/// Tell the daemon to open this window rather than a browser.
///
/// Upstream already has the hook: the tray icon and `goxlr-launcher` both fire
/// an Activate event, and Activate runs whatever `SetActivatorPath` points at,
/// substituting the URL. Without this, clicking the tray would still open a
/// browser tab -- which is the whole thing this binary exists to stop.
///
/// Registering from here rather than from the installer means it is correct
/// after a move, a reinstall, or a run straight out of the build directory,
/// and it costs one request. The daemon persists it, so this is a no-op on
/// every run after the first.
///
/// Best effort throughout. A failure means the tray opens a browser, which is
/// exactly what it did before, so there is nothing here worth interrupting
/// someone over.
fn register_as_activator() {
    let Ok(me) = std::env::current_exe() else {
        return;
    };

    // The daemon takes a path and appends the URL itself, so the value is the
    // executable and nothing else.
    let path = me.to_string_lossy().replace('\\', "\\\\");
    let body = format!(r#"{{"Daemon":{{"SetActivatorPath":"{path}"}}}}"#);

    if let Err(e) = post("/api/command", &body) {
        log::debug!("could not register as the activator: {e}");
    }
}

/// One local HTTP POST, hand-rolled.
///
/// A whole HTTP client for a single fixed request to 127.0.0.1 would be a lot
/// of dependency for a window to carry, and this binary is deliberately thin.
fn post(path: &str, body: &str) -> Result<()> {
    use std::io::{Read, Write};

    let address = SocketAddr::from(([127, 0, 0, 1], PORT));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    write!(
        stream,
        "POST {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{PORT}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )?;

    // Read the status line. Draining the rest matters less than not leaving
    // the daemon writing into a socket nobody is reading.
    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    let status = response.lines().next().unwrap_or_default();
    if !status.contains(" 200") {
        bail!("the daemon answered {status}");
    }
    Ok(())
}

/// Whether anything is listening on the daemon's port.
fn daemon_listening() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], PORT));
    TcpStream::connect_timeout(&address, Duration::from_millis(400)).is_ok()
}

/// Start the daemon if it is not already up, and wait until it answers.
fn ensure_daemon_running() -> Result<()> {
    if daemon_listening() {
        return Ok(());
    }

    let daemon = daemon_path().context(
        "goxlr-daemon.exe was not found next to this program. Attune's window \
         is only a window -- the daemon is what talks to the device.",
    )?;

    std::process::Command::new(&daemon)
        .spawn()
        .with_context(|| format!("could not start {}", daemon.display()))?;

    // Poll rather than sleep a fixed amount: on a warm machine this returns in
    // well under a second, and padding that out would make the app feel slower
    // than it is for the sake of the worst case.
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if daemon_listening() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    bail!(
        "the daemon was started but did not begin serving on port {PORT} within \
         {} seconds",
        STARTUP_TIMEOUT.as_secs()
    )
}

/// The daemon binary, which lives next to this one in an installed copy and in
/// the same target directory when run from a build.
fn daemon_path() -> Option<PathBuf> {
    let here = std::env::current_exe().ok()?;
    let candidate = here.with_file_name("goxlr-daemon.exe");
    candidate.is_file().then_some(candidate)
}

/// Tell someone what went wrong, given there is no console to print to.
#[cfg(windows)]
fn report(message: &str) {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    let wide = |s: &str| {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(once(0))
            .collect::<Vec<u16>>()
    };

    // Declared here rather than taking a dependency on the whole windows crate
    // for one call. MB_ICONERROR | MB_OK.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            hwnd: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }

    let text = wide(message);
    let caption = wide("Attune");
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

#[cfg(not(windows))]
fn report(message: &str) {
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The icon has to decode, or the window silently gets a placeholder --
    /// which is what happens on a failure here, because refusing to open the
    /// application over an icon would be out of proportion. That makes this
    /// the only thing standing between a broken icon and nobody noticing.
    #[test]
    fn the_window_icon_decodes() {
        const ICON: &[u8] = include_bytes!("../resources/attune.png");
        let decoded = image::load_from_memory_with_format(ICON, image::ImageFormat::Png)
            .expect("the bundled icon did not decode");
        let rgba = decoded.into_rgba8();
        let (width, height) = rgba.dimensions();

        assert!(width > 0 && height > 0, "decoded to {width}x{height}");
        eprintln!("icon decoded at {width}x{height}");

        assert!(
            window_icon().is_some(),
            "the icon decoded but tao would not accept it"
        );
    }
}
