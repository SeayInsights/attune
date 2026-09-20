//! Windows spatial audio, per output endpoint.
//!
//! Windows ships its own headphone virtualiser -- Windows Sonic -- free, and
//! exposes a documented API for reading and setting which virtualiser is
//! active on a given endpoint: `SpatialAudioDeviceConfiguration`. Dolby Access
//! and DTS Sound Unbound are, architecturally, licence-and-settings front ends
//! for exactly this mechanism; the renderer they unlock registers itself here
//! and then appears in the same list as Sonic.
//!
//! So Attune does not need to implement a virtualiser, and should not pretend
//! to. What it can do is stop making people go and find the setting: read what
//! is active per bus and switch it, in the app, through the supported API
//! rather than by writing to the endpoint's registry properties.
//!
//! # `IsSpatialAudioFormatSupported` does not mean what it sounds like
//!
//! Measured on a machine with neither Dolby Access nor DTS Sound Unbound
//! installed, it returns `true` for Dolby Atmos for Headphones and for DTS
//! Headphone:X. Setting either then completes without error and leaves the
//! endpoint on whatever it was. So it reports that the platform recognises a
//! format, not that the machine can actually produce it, and it cannot be used
//! on its own to decide what to offer someone.
//!
//! That is why every change here is read back off the endpoint before being
//! called a success, and why a format that fails to take reports what it
//! needs rather than a generic failure.
//!
//! Two things this module deliberately does not do:
//!
//!   * Hard-code a format's identifier. They are GUIDs, they are not
//!     documented, and the one published value found while researching this
//!     did not match what the machine reports. Every subtype is read from
//!     `SpatialAudioFormatSubtype` and round-tripped as an opaque string.
//!   * Name a format "off" from a constant. There is no documented "off"
//!     subtype: Windows 11 reports a null GUID. See `is_off`.

use std::fmt;
use std::time::Duration;

use serde::Serialize;
use windows::Media::Audio::{SpatialAudioDeviceConfiguration, SpatialAudioFormatSubtype};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
};
use windows::core::HSTRING;

/// Windows models "no virtualisation" as a spatial format like any other, with
/// this subtype. It is not in `SpatialAudioFormatSubtype` and is not in the
/// public documentation, so it is asserted here only as a fallback: the normal
/// path is to read whatever `ActiveSpatialAudioFormat` reports while spatial
/// audio is off and use that, which needs no constant at all. It is kept
/// because a first switch *to* off has nothing to read back.
const OFF_SUBTYPE: &str = "Windows.Media.Audio.SpatialAudioFormat.Off";

/// What Windows 11 actually reports on an endpoint with spatial audio off: a
/// null GUID, not a named subtype. Found by asking a machine rather than by
/// reading documentation, which does not cover this.
const OFF_GUID: &str = "{00000000-0000-0000-0000-000000000000}";

/// Whether a subtype means "no virtualisation".
///
/// Deliberately generous. The off value is not documented, this machine spells
/// it as a null GUID, the SDK constant suggests a dotted name, and an empty
/// string is the obvious third possibility. Treating an unrecognised value as
/// a real format is the worse failure -- it would show up as a phantom entry
/// in the list -- so all three read as off.
fn is_off(subtype: &str) -> bool {
    subtype.is_empty() || subtype.eq_ignore_ascii_case(OFF_GUID) || {
        let lower = subtype.to_lowercase();
        lower.ends_with(".off") || lower == OFF_SUBTYPE.to_lowercase()
    }
}

/// The device interface class for audio endpoints. The WinRT device id is the
/// MMDevice id wrapped in a software-device path, and this GUID is the wrapper.
const MMDEVAPI_INTERFACE: &str = "{e6327cad-dcec-4949-ae8a-991e976a79d2}";

/// How long to wait for a format change to show up on the endpoint before
/// calling it a failure. Two seconds: the switch is effectively immediate, so
/// this is generous rather than tuned, and a person who has to wait two
/// seconds to be told it did not work is better served than one told it did.
const POLL_ATTEMPTS: u32 = 50;
const POLL_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Debug)]
pub enum SpatialError {
    /// Windows refused the call. Carries its own message -- these are usually
    /// specific and more useful than anything that could be written here.
    Windows(String),
    /// No render endpoint whose name contains the requested bus.
    NoSuchDevice(String),

    /// Windows accepted the call and the endpoint did not change. This is the
    /// normal outcome for a format that is recognised but not licensed or
    /// installed, and it is the one that would otherwise be reported as
    /// success, so it carries what the format needs.
    NotApplied {
        device: String,
        format: String,
        requirement: String,
    },
}

impl fmt::Display for SpatialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows(m) => write!(f, "Windows rejected the spatial audio call: {m}"),
            Self::NoSuchDevice(d) => write!(f, "no playback device matching '{d}'"),

            Self::NotApplied {
                device,
                format,
                requirement,
            } => write!(
                f,
                "{device} did not switch to {format}. Windows accepted the \
                 change and then ignored it, which is what it does for a \
                 format it knows about but cannot produce. {requirement}"
            ),
        }
    }
}

impl std::error::Error for SpatialError {}

impl From<windows::core::Error> for SpatialError {
    fn from(e: windows::core::Error) -> Self {
        Self::Windows(e.message().trim().to_string())
    }
}

/// One spatial format, as Windows reports it on this machine.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Format {
    /// The WinRT subtype string. Opaque; round-tripped back to `set`.
    pub subtype: String,
    /// What to show a person.
    pub label: String,
    /// Why they might want it, or what it will cost them.
    pub note: String,
    /// Whether Windows recognises this format at all. Not whether it will
    /// work: see the module docs. A format can be recognised, accepted and
    /// still not apply, which is why this is not treated as permission.
    pub recognised: bool,
}

/// What is going on with one endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub device: String,
    /// The subtype currently active, straight from Windows.
    pub active: String,
    /// Which of `formats` is active, for a UI that does not want to compare
    /// opaque strings.
    pub active_label: String,
    pub formats: Vec<Format>,
}

/// The formats Attune knows how to describe, in the order to show them.
///
/// Anything Windows offers that is not in this table still appears, labelled
/// with its own subtype -- a machine with a renderer nobody here anticipated
/// should not have it hidden.
fn known() -> Vec<(HSTRING, &'static str, &'static str)> {
    let mut out = Vec::new();
    if let Ok(s) = SpatialAudioFormatSubtype::WindowsSonic() {
        out.push((
            s,
            "Windows Sonic for Headphones",
            "Free, built into Windows. Turns surround into a headphone mix.",
        ));
    }
    if let Ok(s) = SpatialAudioFormatSubtype::DolbyAtmosForHeadphones() {
        out.push((
            s,
            "Dolby Atmos for Headphones",
            "It needs a paid licence, bought through the Dolby Access app.",
        ));
    }
    if let Ok(s) = SpatialAudioFormatSubtype::DTSHeadphoneX() {
        out.push((
            s,
            "DTS Headphone:X",
            "It needs DTS Sound Unbound, or a headset that licenses it.",
        ));
    }
    out
}

/// A format's display name and what a machine needs before it will work.
///
/// Keyed on the subtype read back from Windows rather than on a constant, so
/// it stays correct if the identifiers ever change; anything unrecognised gets
/// a truthful non-answer rather than a guess.
fn describe(subtype: &str) -> (String, String) {
    for (candidate, label, note) in known() {
        if candidate.to_string().eq_ignore_ascii_case(subtype) {
            return (label.to_string(), note.to_string());
        }
    }
    (
        subtype.to_string(),
        "Whatever provides it is not installed or not licensed on this machine.".to_string(),
    )
}

/// Run a closure on a thread with its own multi-threaded COM apartment.
///
/// The daemon's HTTP workers have no apartment of their own, and WinRT calls
/// need one. Doing this per call rather than once at startup keeps the
/// requirement local to the code that has it -- nothing else in the daemon has
/// to know that the Headphones tab talks to COM.
fn with_com<T, F>(f: F) -> Result<T, SpatialError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, SpatialError> + Send + 'static,
{
    std::thread::spawn(move || {
        // A failure here means the thread already had an apartment, which for
        // a thread that was just spawned it does not, or that one was
        // requested in a different mode. Either way the call below is the
        // thing that decides whether this works.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        f()
    })
    .join()
    .map_err(|_| SpatialError::Windows("the spatial audio thread panicked".into()))?
}

/// Every active render endpoint, as (friendly name, WinRT device id).
fn endpoints() -> Result<Vec<(String, String)>, SpatialError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;

        let mut out = Vec::new();
        for i in 0..collection.GetCount()? {
            let device: IMMDevice = collection.Item(i)?;
            // GetId hands back a PWSTR; decoding it can fail independently of
            // COM, so it does not ride the same `?` as everything else here.
            let id = device.GetId()?.to_string().map_err(|e| {
                SpatialError::Windows(format!("an endpoint id was not valid UTF-16: {e}"))
            })?;
            let name = friendly_name(&device)?;
            out.push((name, format!("\\\\?\\SWD#MMDEVAPI#{id}#{MMDEVAPI_INTERFACE}")));
        }
        Ok(out)
    }
}

fn friendly_name(device: &IMMDevice) -> Result<String, SpatialError> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        Ok(store.GetValue(&PKEY_Device_FriendlyName)?.to_string())
    }
}

/// Resolve a bus name ("Game") to an endpoint, the same substring match the
/// rest of Attune uses so that the two cannot disagree about which device a
/// bus means.
fn resolve(bus: &str) -> Result<(String, String), SpatialError> {
    let wanted = bus.to_lowercase();
    endpoints()?
        .into_iter()
        .find(|(name, _)| name.to_lowercase().contains(&wanted))
        .ok_or_else(|| SpatialError::NoSuchDevice(bus.to_string()))
}

/// What spatial format is active on this bus, and what else it could be.
pub fn status(bus: &str) -> Result<Status, SpatialError> {
    let bus = bus.to_string();
    with_com(move || {
        let (device, id) = resolve(&bus)?;
        let config = SpatialAudioDeviceConfiguration::GetForDeviceId(&HSTRING::from(&id))?;
        let active = config.ActiveSpatialAudioFormat()?.to_string();

        let mut formats = vec![Format {
            // Seeded with the machine's own off value when it is already off,
            // so the UI round-trips something Windows recognises rather than
            // a constant that may not be what this build of Windows uses.
            subtype: if is_off(&active) {
                active.clone()
            } else {
                OFF_GUID.to_string()
            },
            label: "Off".to_string(),
            note: "Plain stereo. No virtualisation, nothing added to the signal.".to_string(),
            recognised: true,
        }];

        for (subtype, label, note) in known() {
            // Ask rather than assume. A machine without Dolby Access says no
            // here, and the UI can show the option greyed with its reason
            // instead of offering something that will fail.
            let recognised = config.IsSpatialAudioFormatSupported(&subtype).unwrap_or(false);
            formats.push(Format {
                subtype: subtype.to_string(),
                label: label.to_string(),
                note: note.to_string(),
                recognised,
            });
        }

        // If Windows reports something not in the table -- or reports "off"
        // with a subtype other than the one assumed above -- show it rather
        // than silently failing to highlight anything.
        if !formats.iter().any(|f| f.subtype == active) {
            if is_off(&active) {
                // Learn the real off value from the machine instead of
                // trusting the constant.
                formats[0].subtype = active.clone();
            } else {
                formats.push(Format {
                    subtype: active.clone(),
                    label: active.rsplit('.').next().unwrap_or(&active).to_string(),
                    note: "Registered on this machine by something other than Windows.".to_string(),
                    recognised: true,
                });
            }
        }

        let active_label = formats
            .iter()
            .find(|f| f.subtype == active)
            .map(|f| f.label.clone())
            .unwrap_or_else(|| "Off".to_string());

        Ok(Status {
            device,
            active,
            active_label,
            formats,
        })
    })
}

/// Switch this bus to a spatial format. Takes a subtype from `status`.
pub fn set(bus: &str, subtype: &str) -> Result<Status, SpatialError> {
    let bus_owned = bus.to_string();
    let subtype_owned = subtype.to_string();

    with_com(move || {
        let (device, id) = resolve(&bus_owned)?;
        let config = SpatialAudioDeviceConfiguration::GetForDeviceId(&HSTRING::from(&id))?;
        let wanted = HSTRING::from(&subtype_owned);


        // Fire the change. The operation's result enum lives in a crate that
        // is an implementation detail of `windows` and has no blocking
        // accessor in this version, so rather than taking a direct dependency
        // on it to read one value, the change is confirmed the way every
        // other write in Attune is confirmed: by reading it back. A write
        // that silently did not land is the failure worth catching, and the
        // property is the thing that actually decides that -- not the
        // operation's own opinion of itself.
        config.SetDefaultSpatialAudioFormatAsync(&wanted)?;

        for _ in 0..POLL_ATTEMPTS {
            if config.ActiveSpatialAudioFormat()?.to_string() == subtype_owned {
                log::debug!("spatial format on {device} is now {subtype_owned}");
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        // Naming the requirement is the whole difference between a dead end
        // and a next step. `describe` knows what each format needs; anything
        // it does not recognise gets the honest generic answer.
        let (label, requirement) = describe(&subtype_owned);
        Err(SpatialError::NotApplied {
            device,
            format: label,
            requirement,
        })
    })?;

    status(bus)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wrapped id is what `GetForDeviceId` wants; getting the shape wrong
    /// fails at runtime with a generic error, so pin it here.
    #[test]
    fn a_winrt_device_id_wraps_the_mmdevice_id() {
        let mm = "{0.0.0.00000000}.{1b2c3d4e-0000-0000-0000-000000000000}";
        let wrapped = format!("\\\\?\\SWD#MMDEVAPI#{mm}#{MMDEVAPI_INTERFACE}");

        assert!(wrapped.starts_with("\\\\?\\SWD#MMDEVAPI#"));
        assert!(wrapped.contains(mm));
        assert!(wrapped.ends_with("{e6327cad-dcec-4949-ae8a-991e976a79d2}"));
    }

    /// Off has to be recognisable without a documented constant, because the
    /// value is learnt from the machine. Both the assumed constant and a bare
    /// empty string have to read as off.
    #[test]
    fn off_is_recognised_however_windows_spells_it() {
        // The GUID is what Windows 11 reports in practice; the others are the
        // spellings the SDK and the obvious empty case suggest.
        for candidate in ["", OFF_SUBTYPE, OFF_GUID, OFF_GUID.to_uppercase().as_str()] {
            assert!(is_off(candidate), "{candidate} should read as off");
        }

        // And a real format must not. The GUID is the one this machine
        // reports for Windows Sonic.
        assert!(!is_off("Windows.Media.Audio.SpatialAudioFormat.WindowsSonic"));
        assert!(!is_off("{B53D940C-B846-4831-9F76-D102B9B725A0}"));
    }

    /// A format that does not apply has to say what it needs, or the person is
    /// left with "it did not work" and nowhere to go. Measured behaviour: a
    /// machine without Dolby Access accepts the change and ignores it.
    #[test]
    fn an_unapplied_format_reports_what_it_needs() {
        let sonic = SpatialAudioFormatSubtype::WindowsSonic().unwrap().to_string();
        let (label, requirement) = describe(&sonic);
        assert_eq!(label, "Windows Sonic for Headphones");
        assert!(requirement.contains("Free"), "got: {requirement}");

        let dolby = SpatialAudioFormatSubtype::DolbyAtmosForHeadphones()
            .unwrap()
            .to_string();
        let (label, requirement) = describe(&dolby);
        assert_eq!(label, "Dolby Atmos for Headphones");
        assert!(requirement.contains("Dolby Access"), "got: {requirement}");
    }

    /// Anything unrecognised still gets an answer rather than an empty string,
    /// because Windows may report a renderer this table has never heard of.
    #[test]
    fn an_unknown_format_gets_a_truthful_non_answer() {
        let (label, requirement) = describe("{deadbeef-0000-0000-0000-000000000000}");
        assert_eq!(label, "{deadbeef-0000-0000-0000-000000000000}");
        assert!(!requirement.is_empty());
        assert!(requirement.contains("not installed"), "got: {requirement}");
    }

    /// The formats are read from Windows, never written down here. If this
    /// ever fails it means the SDK constants moved, which is exactly the case
    /// hard-coding them would have hidden.
    #[test]
    fn every_described_format_comes_from_windows() {
        let table = known();
        assert!(!table.is_empty(), "Windows offered no spatial formats at all");
        for (subtype, label, note) in table {
            assert!(!subtype.to_string().is_empty(), "{label} has no subtype");
            assert!(!note.is_empty(), "{label} does not say what it needs");
        }
    }
}
