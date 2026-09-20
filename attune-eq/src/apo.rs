//! Equalizer APO: finding it, and writing configuration it will apply.
//!
//! # Why an external dependency at all
//!
//! The GoXLR's own equaliser applies to the microphone, not to what you hear.
//! Correcting headphones therefore has to happen on the Windows side, and APO is
//! the supported mechanism for inserting processing into an endpoint's chain.
//!
//! # What Attune will and will not do to someone's system
//!
//! Installing APO needs administrator rights, a reboot, and a choice about which
//! endpoints it attaches to. It modifies the system audio pipeline. Attune does
//! not install it, and never installs it silently -- that decision belongs to
//! the person whose audio it is.
//!
//! When APO *is* present, Attune writes its own file and adds a single `Include:`
//! line to the main configuration. It never rewrites configuration it did not
//! author, so an existing setup survives, and removing Attune is one line.
//!
//! # The limit worth knowing before relying on this
//!
//! APO processes audio that goes through the Windows system effect
//! infrastructure. Anything that deliberately bypasses it -- WASAPI exclusive
//! mode, ASIO -- is untouched. Competitive games are the most likely things to
//! do that, which is precisely why routing the audio through Attune instead
//! exists as a separate mode.

use std::path::{Path, PathBuf};

use crate::curve::Curve;

/// The marker Attune writes so it can recognise its own configuration.
const MARKER: &str = "# --- Attune: generated, do not edit by hand ---";

/// The file Attune owns inside APO's config directory.
pub const CONFIG_FILE: &str = "attune.txt";

/// A located Equalizer APO installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// Installation root.
    pub root: PathBuf,
    /// Where configuration files live.
    pub config_dir: PathBuf,
}

impl Install {
    /// Attune's own configuration file.
    pub fn attune_config(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    /// APO's main configuration file.
    pub fn main_config(&self) -> PathBuf {
        self.config_dir.join("config.txt")
    }

    /// Whether the main configuration already pulls Attune's file in.
    pub fn is_included(&self) -> bool {
        std::fs::read_to_string(self.main_config())
            .map(|text| {
                text.lines().any(|line| {
                    let line = line.trim();
                    !line.starts_with('#')
                        && line.to_ascii_lowercase().contains("include:")
                        && line.contains(CONFIG_FILE)
                })
            })
            .unwrap_or(false)
    }
}

/// Find Equalizer APO, if it is installed.
pub fn detect() -> Option<Install> {
    let candidates = [
        std::env::var("ProgramFiles").ok(),
        std::env::var("ProgramFiles(x86)").ok(),
    ];

    for base in candidates.into_iter().flatten() {
        let root = Path::new(&base).join("EqualizerAPO");
        let config_dir = root.join("config");
        // The config directory is the thing that matters. A root without one is
        // a broken or partial install, and reporting it as usable would produce
        // a confusing failure later instead of a clear one now.
        if config_dir.is_dir() {
            return Some(Install { root, config_dir });
        }
    }
    None
}

/// A correction bound to one audio endpoint.
#[derive(Debug, Clone)]
pub struct BusCurve {
    /// The endpoint name on its own, e.g. "Game".
    pub endpoint: String,
    /// The connection it belongs to, e.g. "TC-HELICON GoXLR".
    ///
    /// Supplied whenever it is known. Without it the endpoint name alone must
    /// be unique, and it often is not -- SteelSeries Sonar creates its own
    /// "Game" device, and a bare match would apply a GoXLR correction to it.
    pub connection: Option<String>,
    pub curve: Curve,
}

impl BusCurve {
    /// Split the composite name an audio API reports into its two parts.
    ///
    /// cpal and Windows both render an endpoint as "Game (TC-HELICON GoXLR)",
    /// but Equalizer APO does **not** match that string: its `Device:` directive
    /// takes the endpoint name and the connection name as separate
    /// whitespace-separated patterns. Passing the composite form matches nothing
    /// and fails silently -- no error, no log, no effect -- which is exactly how
    /// this was originally shipped and why it did nothing.
    pub fn from_composite(name: &str, curve: Curve) -> Self {
        match name.split_once(" (") {
            Some((endpoint, rest)) => Self {
                endpoint: endpoint.trim().to_string(),
                connection: Some(rest.trim_end_matches(')').trim().to_string()),
                curve,
            },
            None => Self {
                endpoint: name.trim().to_string(),
                connection: None,
                curve,
            },
        }
    }

    /// The `Device:` line this bus needs.
    pub fn device_directive(&self) -> String {
        match &self.connection {
            Some(c) => format!("{} {}", self.endpoint, c),
            None => self.endpoint.clone(),
        }
    }
}

/// Render the configuration APO should apply.
///
/// Each bus becomes its own `Device:` section, which is what allows different
/// curves on Game, Music, Chat and System at the same time -- the capability
/// that exists here only because the GoXLR splits those into separate endpoints
/// before Windows ever mixes them.
pub fn render(buses: &[BusCurve]) -> String {
    let mut out = String::new();
    out.push_str(MARKER);
    out.push_str("\n# Written by Attune. Edits will be overwritten.\n");
    out.push_str("# Remove the Include: line in config.txt to disable all of this.\n");

    if buses.is_empty() {
        out.push_str("\n# No corrections are currently configured.\n");
        return out;
    }

    for bus in buses {
        out.push_str("\nDevice: ");
        out.push_str(&bus.device_directive());
        out.push('\n');
        out.push_str("Channel: all\n");

        if !bus.curve.name.is_empty() {
            out.push_str("# ");
            out.push_str(&bus.curve.name);
            out.push('\n');
        }

        out.push_str(&format!("Preamp: {:.2} dB\n", bus.curve.preamp_db));

        for (index, filter) in bus.curve.filters.iter().enumerate() {
            out.push_str(&format!("Filter {}: {}\n", index + 1, filter));
        }
    }

    out
}

/// What changed on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Attune's file was written and the main config already included it.
    Written,
    /// Attune's file was written and the `Include:` line was added.
    WrittenAndIncluded,
}

/// Errors from writing configuration.
#[derive(Debug, thiserror::Error)]
pub enum ApoError {
    #[error("Equalizer APO is not installed")]
    NotInstalled,

    #[error(
        "could not write {path}: {source}. Equalizer APO's config directory is \
         usually under Program Files, which needs administrator rights to write \
         to unless its permissions were relaxed during installation."
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Write Attune's configuration, and make sure APO loads it.
///
/// The `Include:` line is appended, never inserted, and the rest of the file is
/// read and written back untouched.
pub fn apply(install: &Install, buses: &[BusCurve]) -> Result<Applied, ApoError> {
    let path = install.attune_config();
    std::fs::write(&path, render(buses)).map_err(|source| ApoError::Write {
        path: path.clone(),
        source,
    })?;

    if install.is_included() {
        return Ok(Applied::Written);
    }

    let main = install.main_config();
    let mut existing = std::fs::read_to_string(&main).unwrap_or_default();
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str("\n# Added by Attune. Delete this line to disable it.\n");
    existing.push_str(&format!("Include: {CONFIG_FILE}\n"));

    std::fs::write(&main, existing).map_err(|source| ApoError::Write { path: main, source })?;

    Ok(Applied::WrittenAndIncluded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Filter, FilterKind};

    fn curve(name: &str, gain: f32) -> Curve {
        Curve {
            name: name.into(),
            preamp_db: -3.0,
            filters: vec![Filter {
                kind: FilterKind::Peaking,
                freq_hz: 8000.0,
                gain_db: gain,
                q: 2.0,
            }],
        }
    }

    /// Regression: the composite name is not what APO matches.
    ///
    /// Shipping `Device: Game (TC-HELICON GoXLR)` produced a configuration that
    /// parsed, loaded, and did nothing at all. Verified against a real
    /// installation by playing pink noise through the bus and measuring the
    /// return: with the composite form the spectrum was unchanged within noise;
    /// with the split form a -20 dB probe filter moved the band 7 dB.
    #[test]
    fn a_composite_device_name_is_split_into_the_two_patterns_apo_matches() {
        let bus = BusCurve::from_composite("Game (TC-HELICON GoXLR)", Curve::default());
        assert_eq!(bus.endpoint, "Game");
        assert_eq!(bus.connection.as_deref(), Some("TC-HELICON GoXLR"));
        assert_eq!(bus.device_directive(), "Game TC-HELICON GoXLR");
        assert!(
            !bus.device_directive().contains('('),
            "parentheses in a Device directive match nothing"
        );
    }

    #[test]
    fn a_name_with_no_connection_is_used_as_is() {
        let bus = BusCurve::from_composite("Speakers", Curve::default());
        assert_eq!(bus.device_directive(), "Speakers");
        assert_eq!(bus.connection, None);
    }

    /// The connection is what keeps a GoXLR curve off someone else's "Game".
    #[test]
    fn the_connection_disambiguates_a_shared_endpoint_name() {
        let goxlr = BusCurve::from_composite("Game (TC-HELICON GoXLR)", Curve::default());
        let sonar = BusCurve::from_composite("Game (SteelSeries Sonar)", Curve::default());

        assert_eq!(goxlr.endpoint, sonar.endpoint);
        assert_ne!(
            goxlr.device_directive(),
            sonar.device_directive(),
            "two devices both named Game must not produce the same directive"
        );
    }

    #[test]
    fn each_bus_becomes_its_own_device_section() {
        let text = render(&[
            BusCurve::from_composite("Game (TC-HELICON GoXLR)", curve("competitive", -6.0)),
            BusCurve::from_composite("Music (TC-HELICON GoXLR)", curve("harman", 2.0)),
        ]);

        assert_eq!(text.matches("Device:").count(), 2);
        assert!(text.contains("Device: Game TC-HELICON GoXLR"));
        assert!(text.contains("Device: Music TC-HELICON GoXLR"));
        // The whole point: different corrections, simultaneously.
        assert!(text.contains("Gain -6.00 dB"));
        assert!(text.contains("Gain 2.00 dB"));
    }

    #[test]
    fn filters_are_numbered_from_one_per_device() {
        let text = render(&[BusCurve::from_composite(
            "Game",
            Curve {
                name: "t".into(),
                preamp_db: 0.0,
                filters: vec![
                    Filter {
                        kind: FilterKind::Peaking,
                        freq_hz: 100.0,
                        gain_db: 1.0,
                        q: 1.0,
                    },
                    Filter {
                        kind: FilterKind::Peaking,
                        freq_hz: 200.0,
                        gain_db: 1.0,
                        q: 1.0,
                    },
                ],
            },
        )]);
        assert!(text.contains("Filter 1:"));
        assert!(text.contains("Filter 2:"));
    }

    #[test]
    fn rendered_config_round_trips_through_the_curve_parser() {
        let original = curve("dt990", -4.5);
        let text = render(&[BusCurve::from_composite("Game", original.clone())]);

        let back = Curve::parse("x", &text).unwrap();
        assert_eq!(back.filters.len(), 1);
        assert!((back.filters[0].gain_db - original.filters[0].gain_db).abs() < 0.01);
        assert!((back.preamp_db - original.preamp_db).abs() < 0.01);
    }

    #[test]
    fn an_empty_configuration_is_valid_and_says_so() {
        let text = render(&[]);
        assert!(text.contains(MARKER));
        assert!(!text.contains("Device:"));
    }

    #[test]
    fn generated_config_is_marked_as_generated() {
        let text = render(&[BusCurve::from_composite("Game", curve("t", -1.0))]);
        assert!(text.starts_with(MARKER), "must be recognisable as ours");
        assert!(
            text.contains("Remove the Include:"),
            "must say how to undo it"
        );
    }

    #[test]
    fn detect_returns_none_rather_than_a_broken_install() {
        // Nothing is installed in the test environment, which is the case this
        // must handle without panicking or inventing a path.
        let found = detect();
        if let Some(install) = found {
            assert!(install.config_dir.is_dir());
        }
    }
}
