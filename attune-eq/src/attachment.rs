//! Whether Equalizer APO is still actually in the audio path.
//!
//! # The failure this exists for
//!
//! Equalizer APO works by reconfiguring an audio driver's effect chain after
//! the driver was installed -- it writes its own CLSIDs into each endpoint's
//! `FxProperties` and remembers what it displaced. Its own maintainer
//! documents that this is not supported by Microsoft and that the
//! configuration "will be overwritten every time the audio driver is
//! reinstalled, and some Windows updates have a tendency to reinstall audio
//! drivers."
//!
//! So one day, without anyone doing anything, the GoXLR driver gets
//! reinstalled and every correction Attune applied stops running. Nothing
//! errors. The files are all still on disk, the UI still shows the curves, and
//! the audio is quietly flat. That is the worst shape a failure can take: it
//! looks exactly like success.
//!
//! # How it is detected
//!
//! Two records disagree.
//!
//! `HKLM\SOFTWARE\EqualizerAPO\Child APOs\{endpoint}` is APO's own note of
//! where it installed itself, and of which CLSIDs it displaced to get there.
//! The endpoint's `FxProperties` is what Windows will actually load. A driver
//! reinstall rewrites the second and leaves the first alone, because the first
//! is not the driver's to touch.
//!
//! So: an endpoint that APO has a `Child APOs` entry for, but whose
//! `FxProperties` no longer names an Equalizer APO CLSID, has been detached.
//! That is a precise, cheap, read-only check, and it does not depend on
//! anything about Attune -- it would detect the same wipe for someone using
//! Equalizer APO on its own.
//!
//! # What this module will not do
//!
//! Repair it. Rewriting another program's effect-chain registration is how
//! this class of breakage happens in the first place, and doing it wrong
//! silently removes whatever displaced APO. Attune reports the state and says
//! which tool fixes it; reinstalling APO's own configurator is the supported
//! path and it is the one that knows what to restore.

use std::collections::BTreeMap;

use serde::Serialize;

/// Equalizer APO's effect CLSIDs. An endpoint referencing any of these has APO
/// in its chain.
///
/// Compared case-insensitively: the registry is inconsistent about the case of
/// hex digits in a GUID, and on this machine a single endpoint carries both
/// spellings.
const APO_CLSIDS: &[&str] = &[
    // EqualizerAPO Pre-Mix Class (stream effects).
    "{EACD2258-FCAC-4FF4-B36D-419E924A6D79}",
    // EqualizerAPO Post-Mix Class (endpoint effects).
    "{EC1CC9CE-FAED-4822-828A-82A81A6F018F}",
];

/// The property-store keys that name an endpoint's effect CLSIDs: the stream,
/// mode and endpoint effect slots respectively.
const FX_CLSID_KEYS: &[&str] = &[
    "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},5",
    "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},6",
    "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},7",
];

const CHILD_APOS: &str = r"SOFTWARE\EqualizerAPO\Child APOs";
const MMDEVICES: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio";
/// PKEY_Device_FriendlyName, as the registry spells it.
const FRIENDLY_NAME: &str = "{a45c254e-df1c-4efd-8020-67d146a850e0},2";

/// One endpoint Equalizer APO believes it is installed on.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Endpoint {
    /// The endpoint GUID, which is also the registry key name.
    pub id: String,
    /// The friendly name, when the endpoint still exists. A `Child APOs` entry
    /// can outlive the device it refers to -- unplugging an interface leaves
    /// the entry behind -- and that is not a wipe.
    pub name: Option<String>,
    /// Whether the endpoint still loads Equalizer APO.
    pub attached: bool,
}

/// What the check found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Attachment {
    /// Whether Equalizer APO has any registration at all. False means it is
    /// not installed, which is a different problem with a different fix.
    pub installed: bool,
    /// Every endpoint APO claims, attached or not.
    pub endpoints: Vec<Endpoint>,
    /// The endpoints APO claims and no longer runs on. Empty is the good case.
    pub detached: Vec<String>,
}

impl Attachment {
    /// Whether anything needs saying to a person.
    pub fn healthy(&self) -> bool {
        self.detached.is_empty()
    }

    /// A sentence for someone who has not read any of this.
    pub fn explain(&self) -> String {
        if !self.installed {
            return "Equalizer APO is not installed, so no correction is running.".to_string();
        }
        if self.healthy() {
            return "Equalizer APO is attached to every endpoint it was set up on.".to_string();
        }

        // Naming the endpoints matters. "Something is wrong" sends someone
        // looking; "Game and Music are detached" tells them what they will
        // hear and where.
        let names = self.detached.join(", ");
        format!(
            "Equalizer APO is no longer attached to {names}. Everything on \
             those is running flat, whatever the curves say. This happens when \
             an audio driver is reinstalled, which Windows Update sometimes \
             does on its own -- reinstalling Equalizer APO and re-selecting \
             these devices puts it back."
        )
    }
}

/// Check whether APO is still in the path everywhere it thinks it is.
///
/// Read-only, and cheap enough to run whenever the Headphones tab is opened.
#[cfg(windows)]
pub fn check() -> Attachment {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    let Ok(children) = hklm.open_subkey_with_flags(CHILD_APOS, KEY_READ) else {
        return Attachment {
            installed: false,
            endpoints: Vec::new(),
            detached: Vec::new(),
        };
    };

    let known = endpoint_names(&hklm);
    let mut endpoints = Vec::new();
    let mut detached = Vec::new();

    for id in children.enum_keys().flatten() {
        let name = known.get(&id.to_lowercase()).cloned();
        let attached = is_attached(&hklm, &id);

        // An entry whose endpoint has gone is not a wipe -- it is an unplugged
        // device, or one APO was installed on and Windows has since forgotten.
        // Reporting that as breakage would cry wolf until nobody looked.
        if !attached && name.is_some() {
            detached.push(name.clone().unwrap_or_else(|| id.clone()));
        }

        endpoints.push(Endpoint { id, name, attached });
    }

    endpoints.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    detached.sort();

    Attachment {
        installed: true,
        endpoints,
        detached,
    }
}

#[cfg(not(windows))]
pub fn check() -> Attachment {
    Attachment {
        installed: false,
        endpoints: Vec::new(),
        detached: Vec::new(),
    }
}

/// Whether one endpoint's effect chain still names Equalizer APO.
#[cfg(windows)]
fn is_attached(hklm: &winreg::RegKey, id: &str) -> bool {
    use winreg::enums::KEY_READ;

    // Render and capture are separate trees and an endpoint is in exactly one,
    // so both are tried rather than assumed. Attune corrects output buses, but
    // APO is commonly installed on the microphone too, and a check that
    // ignored capture would report a mic wipe as healthy.
    for tree in ["Render", "Capture"] {
        let path = format!("{MMDEVICES}\\{tree}\\{id}\\FxProperties");
        let Ok(fx) = hklm.open_subkey_with_flags(&path, KEY_READ) else {
            continue;
        };
        for key in FX_CLSID_KEYS {
            let Ok(value) = fx.get_value::<String, _>(key) else {
                continue;
            };
            if APO_CLSIDS
                .iter()
                .any(|clsid| clsid.eq_ignore_ascii_case(value.trim()))
            {
                return true;
            }
        }
    }
    false
}

/// Every endpoint Windows currently knows, by lowercased GUID.
#[cfg(windows)]
fn endpoint_names(hklm: &winreg::RegKey) -> BTreeMap<String, String> {
    use winreg::enums::KEY_READ;

    let mut out = BTreeMap::new();
    for tree in ["Render", "Capture"] {
        let Ok(root) = hklm.open_subkey_with_flags(format!("{MMDEVICES}\\{tree}"), KEY_READ) else {
            continue;
        };
        for id in root.enum_keys().flatten() {
            let Ok(props) = root.open_subkey_with_flags(format!("{id}\\Properties"), KEY_READ)
            else {
                continue;
            };
            if let Ok(name) = props.get_value::<String, _>(FRIENDLY_NAME) {
                out.insert(id.to_lowercase(), name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(name: &str, attached: bool) -> Endpoint {
        Endpoint {
            id: format!("{{{name}}}"),
            name: Some(name.to_string()),
            attached,
        }
    }

    #[test]
    fn a_healthy_install_says_so_without_alarm() {
        let a = Attachment {
            installed: true,
            endpoints: vec![endpoint("Game", true), endpoint("Music", true)],
            detached: vec![],
        };
        assert!(a.healthy());
        assert!(a.explain().contains("attached to every endpoint"));
    }

    /// The explanation has to name the endpoints. "Something is wrong" sends
    /// someone looking; naming them says what they will hear and where.
    #[test]
    fn a_wipe_names_the_endpoints_and_the_fix() {
        let a = Attachment {
            installed: true,
            endpoints: vec![endpoint("Game", false), endpoint("Music", true)],
            detached: vec!["Game".to_string()],
        };
        assert!(!a.healthy());

        let text = a.explain();
        assert!(text.contains("Game"), "does not name the endpoint: {text}");
        assert!(!text.contains("Music"), "names a healthy endpoint: {text}");
        assert!(
            text.contains("running flat"),
            "does not say what it sounds like: {text}"
        );
        assert!(
            text.contains("reinstall"),
            "does not say how to fix it: {text}"
        );
    }

    /// Not installed is a different problem with a different fix, and must not
    /// be reported as a wipe.
    #[test]
    fn a_missing_install_is_not_reported_as_a_wipe() {
        let a = Attachment {
            installed: false,
            endpoints: vec![],
            detached: vec![],
        };
        assert!(a.healthy());
        assert!(a.explain().contains("not installed"));
    }

    /// The negative branch of `is_attached`, exercised on real data.
    ///
    /// A wipe cannot be staged without editing the registry, so this goes the
    /// other way: an endpoint Equalizer APO was never installed on must read
    /// as not attached. Any machine has plenty of those. Without this, the
    /// code path that reports a wipe never runs until the day it matters.
    #[test]
    #[cfg(windows)]
    fn an_endpoint_apo_never_touched_reads_as_not_attached() {
        use winreg::RegKey;
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let Ok(children) = hklm.open_subkey_with_flags(CHILD_APOS, KEY_READ) else {
            return; // Equalizer APO is not installed; nothing to compare against.
        };
        let claimed: Vec<String> = children
            .enum_keys()
            .flatten()
            .map(|k| k.to_lowercase())
            .collect();

        let mut checked = 0;
        for (id, name) in endpoint_names(&hklm) {
            if claimed.contains(&id) {
                continue;
            }
            assert!(
                !is_attached(&hklm, &id),
                "{name} carries an Equalizer APO CLSID but APO has no record of it"
            );
            checked += 1;
        }

        assert!(
            checked > 0,
            "no unclaimed endpoints to check -- the negative path went untested"
        );
    }

    /// Run against the real registry. Asserts only what must hold on any
    /// machine, because what is installed is not this test's business.
    #[test]
    #[cfg(windows)]
    fn the_real_check_is_self_consistent() {
        let a = check();

        if !a.installed {
            assert!(a.endpoints.is_empty());
            assert!(a.detached.is_empty());
            return;
        }

        // Every detached name must correspond to an endpoint that is both
        // known to Windows and not attached. A name in `detached` that is not
        // in `endpoints` would be a report with nothing behind it.
        for name in &a.detached {
            assert!(
                a.endpoints
                    .iter()
                    .any(|e| e.name.as_deref() == Some(name) && !e.attached),
                "{name} is reported detached but has no matching endpoint"
            );
        }

        // And nothing attached may be listed as detached.
        for endpoint in &a.endpoints {
            if endpoint.attached
                && let Some(name) = &endpoint.name
            {
                assert!(
                    !a.detached.contains(name),
                    "{name} is both attached and not"
                );
            }
        }
    }
}
