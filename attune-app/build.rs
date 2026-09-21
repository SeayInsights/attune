use std::io::Error;

#[cfg(target_os = "windows")]
use windres::Build;

/// Embed the icon, so the window and the taskbar show Attune rather than the
/// generic executable icon. Same mechanism upstream uses for the daemon and
/// launcher.
fn main() -> Result<(), Error> {
    #[cfg(target_os = "windows")]
    {
        Build::new().compile("./resources/attune-app.rc").unwrap();
    }

    Ok(())
}
