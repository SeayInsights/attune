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

use crate::crossfeed::Crossfeed;
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
    /// Processing that runs after the correction, in the order it is applied.
    pub extras: Extras,
}

/// Everything a bus can carry besides its equaliser curve.
///
/// Kept as one struct rather than loose fields so that adding the next one
/// does not touch every construction site.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extras {
    /// Stereo crossfeed, applied after the correction so it acts on the
    /// corrected signal rather than fighting it.
    pub crossfeed: Option<Crossfeed>,
    /// A VST plugin to run at the end of the chain. Attune ships none; this is
    /// how someone brings their own compressor for loudness levelling, which
    /// APO's own config language cannot express.
    pub plugin: Option<String>,
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
                extras: Extras::default(),
            },
            None => Self {
                endpoint: name.trim().to_string(),
                connection: None,
                curve,
                extras: Extras::default(),
            },
        }
    }

    /// Attach post-correction processing.
    pub fn with_extras(mut self, extras: Extras) -> Self {
        self.extras = extras;
        self
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

        // Order matters and is deliberate. Crossfeed acts on the corrected
        // signal, so it comes after the filters -- crossfeeding first would
        // mean the correction is applied to a signal that has already been
        // blended, and the two would fight over the same frequencies. The
        // plugin goes last because a compressor should see the finished mix.
        if let Some(crossfeed) = &bus.extras.crossfeed {
            out.push_str(&render_crossfeed(crossfeed));
        }
        if let Some(plugin) = &bus.extras.plugin {
            out.push_str(&render_plugin(plugin));
        }
    }

    out
}

/// The directive Attune writes to load a VST plugin.
///
/// APO silently ignores any directive it does not recognise -- verified by
/// feeding it an invented one and watching it produce no trace line and no
/// error. So this name cannot be trusted on faith, and [`plugin_loaded`]
/// exists to confirm it took.
pub const PLUGIN_DIRECTIVE: &str = "VSTPlugin";

/// The trace line APO emits when a plugin does load, used to verify the above.
pub const PLUGIN_TRACE: &str = "Adding VST plugin";

fn render_plugin(plugin: &str) -> String {
    format!(
        "\n# Loudness levelling. Attune ships no plugin -- this is the one you \
         chose.\n{PLUGIN_DIRECTIVE}: {plugin}\n"
    )
}

/// Render crossfeed as APO directives.
///
/// The shape: copy both channels aside, low-pass and delay the copies, mix each
/// copy into the *opposite* original, then take back the level the sum added.
/// Verified against a real installation with Equalizer APO's own Benchmark
/// tool, which traces every directive it accepts -- every line below appears in
/// that trace.
fn render_crossfeed(crossfeed: &Crossfeed) -> String {
    let c = crossfeed.sane();
    let g = c.mix_factor();

    let mut out = String::new();
    out.push_str(&format!(
        "\n# Crossfeed: {:.1} dB down, below {:.0} Hz, {:.0} us late\n",
        c.level_db, c.cutoff_hz, c.delay_us
    ));

    // Virtual channels. APO creates these on assignment and discards them at
    // the end, so they cost nothing but a name.
    out.push_str("Copy: XL=L XR=R\n");
    out.push_str("Channel: XL XR\n");
    out.push_str(&format!("Filter: ON LP Fc {:.0} Hz\n", c.cutoff_hz));
    if c.delay_us > 0.0 {
        out.push_str(&format!("Delay: {:.3} ms\n", c.delay_ms()));
    }

    // Each ear gets the *other* channel's shadowed copy. Crossing these over
    // would widen the image instead of narrowing it.
    out.push_str("Channel: L\n");
    out.push_str(&format!("Copy: L=L+{g:.4}*XR\n"));
    out.push_str("Channel: R\n");
    out.push_str(&format!("Copy: R=R+{g:.4}*XL\n"));

    out.push_str("Channel: all\n");
    out.push_str(&format!("Preamp: {:.2} dB\n", c.headroom_db()));
    out
}

/// The configuration to write when Attune is bypassed.
///
/// Not an empty file and not a deleted one: APO reloads on change, so leaving a
/// file it is told to include missing is a different state from one that is
/// present and does nothing. This says plainly what it is, so anyone reading
/// the config directory finds an explanation rather than a mystery.
pub fn render_bypassed() -> String {
    let mut out = String::new();
    out.push_str(MARKER);
    out.push_str("\n# Written by Attune. Edits will be overwritten.\n");
    out.push_str("#\n");
    out.push_str("# Attune is bypassed. Nothing here is being applied, so you are\n");
    out.push_str("# hearing the channels as they come out of the GoXLR. Unbypass in\n");
    out.push_str("# the Headphones tab to put the corrections back.\n");
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

/// Write the bypassed configuration, leaving the `Include:` line alone.
///
/// Bypass has to be reversible without re-deriving anything, and it has to take
/// effect immediately -- it exists so you can hold a button and hear the
/// difference. APO watches its config directory and reloads on change, so
/// rewriting one file is the whole mechanism.
pub fn apply_bypassed(install: &Install) -> Result<Applied, ApoError> {
    let path = install.attune_config();
    std::fs::write(&path, render_bypassed()).map_err(|source| ApoError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(Applied::Written)
}

/// Ask Equalizer APO's own Benchmark tool what it made of the current config.
///
/// Benchmark loads the real configuration, traces every directive it accepted,
/// and processes a sweep through it, all without touching live audio. That
/// makes it the only way to find out whether a directive did anything, which
/// matters because APO silently ignores what it does not understand: an
/// invented directive and a plugin that failed to load are indistinguishable
/// from the outside, and both look exactly like success.
///
/// `None` means Benchmark could not be run at all, which is not the same as
/// "the directive was rejected" and must not be reported as one.
pub fn trace_config(install: &Install, endpoint: &str, connection: Option<&str>) -> Option<String> {
    let exe = install.root.join("Benchmark.exe");
    if !exe.is_file() {
        return None;
    }

    let mut command = std::process::Command::new(exe);
    command
        .arg("--nopause")
        .arg("-v")
        // A one-second sweep. The trace happens at config load, so the audio
        // is beside the point -- this is just the shortest run that produces
        // one.
        .arg("-l")
        .arg("1")
        .arg("-o")
        .arg(std::env::temp_dir().join("attune-trace.wav"))
        .arg("--devicename")
        .arg(endpoint);

    if let Some(connection) = connection {
        command.arg("--connectionname").arg(connection);
    }

    #[cfg(windows)]
    {
        // Without this a console window flashes up on every check.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command.output().ok()?;

    // The trace goes to stderr; the benchmark results go to stdout. Both are
    // wanted, because which stream carries what is not worth depending on.
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    Some(text)
}

/// Whether APO actually loaded the plugin for this endpoint.
///
/// `None` means it could not be determined, which is reported as such rather
/// than guessed either way.
pub fn plugin_loaded(install: &Install, endpoint: &str, connection: Option<&str>) -> Option<bool> {
    trace_config(install, endpoint, connection).map(|text| text.contains(PLUGIN_TRACE))
}

/// Configuration loaded before Attune's, that will colour everything it does.
///
/// APO applies `config.txt` top to bottom, and Attune's `Include:` is appended
/// at the end, so anything above it is already in the signal by the time a
/// correction runs. A stock install ships `Preamp: -6 dB` and an
/// `Include: example.txt` that boosts 20 Hz by 4 dB -- measure a corrected bus
/// with those active and the result will not match the curve on screen, with
/// nothing on screen to explain why.
pub fn interference(install: &Install) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(install.main_config()) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // Everything from Attune's own include onwards is Attune's business.
        if line.to_ascii_lowercase().contains("include:") && line.contains(CONFIG_FILE) {
            break;
        }

        let lower = line.to_ascii_lowercase();
        if lower.starts_with("preamp:") {
            // A preamp of zero changes nothing and is not worth mentioning.
            if !line
                .split(':')
                .nth(1)
                .and_then(|v| v.trim().trim_end_matches(" dB").trim().parse::<f32>().ok())
                .is_some_and(|db| db.abs() < 0.05)
            {
                found.push(line.to_string());
            }
        } else if lower.starts_with("include:")
            || lower.starts_with("filter")
            || lower.starts_with("graphiceq:")
        {
            // A flat graphic equaliser is the APO default and is harmless.
            if lower.starts_with("graphiceq:") && is_flat_graphic_eq(line) {
                continue;
            }
            found.push(line.to_string());
        }
    }
    found
}

/// Comment out everything loading before Attune's own include.
///
/// The lines are commented rather than deleted, and the original file is kept
/// alongside as `config.txt.attune-backup`. This is somebody else's
/// configuration: they may have put that preamp there on purpose, and a tool
/// that silently deletes what it does not recognise is a tool you cannot trust
/// with a config directory.
///
/// Returns the lines that were commented.
pub fn silence_interference(install: &Install) -> Result<Vec<String>, ApoError> {
    let main = install.main_config();
    let text = std::fs::read_to_string(&main).map_err(|source| ApoError::Write {
        path: main.clone(),
        source,
    })?;

    let offending = interference(install);
    if offending.is_empty() {
        return Ok(Vec::new());
    }

    // Keep a copy before touching anything. One backup, not a numbered series:
    // the point is to be able to get back to how it was, and a directory full
    // of backups is its own mess.
    let backup = main.with_extension("txt.attune-backup");
    if !backup.exists() {
        std::fs::copy(&main, &backup).map_err(|source| ApoError::Write {
            path: backup.clone(),
            source,
        })?;
    }

    let mut out = String::new();
    let mut commented = Vec::new();
    for line in text.lines() {
        if offending.iter().any(|o| o == line.trim()) && !line.trim_start().starts_with('#') {
            out.push_str("# Commented out by Attune -- it was changing the sound before\n");
            out.push_str("# Attune's own settings were applied. Delete these two lines and\n");
            out.push_str("# uncomment below to put it back.\n# ");
            out.push_str(line);
            out.push('\n');
            commented.push(line.trim().to_string());
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }

    std::fs::write(&main, out).map_err(|source| ApoError::Write { path: main, source })?;
    Ok(commented)
}

/// Whether a `GraphicEQ:` line is all zeroes, which is APO's own default.
fn is_flat_graphic_eq(line: &str) -> bool {
    let Some(values) = line.split_once(':').map(|(_, v)| v) else {
        return false;
    };
    values
        .split(';')
        .filter(|point| !point.trim().is_empty())
        .all(|point| {
            point
                .split_whitespace()
                .nth(1)
                .and_then(|gain| gain.parse::<f32>().ok())
                .is_some_and(|gain| gain.abs() < 0.05)
        })
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

    /// The crossed-over assignment is the whole point, and it is the easy
    /// thing to get backwards: L must receive the copy of R, not of L.
    /// Uncrossing it would widen the image instead of narrowing it, and would
    /// still parse, still load, and still sound like something.
    #[test]
    fn crossfeed_sends_each_channel_to_the_opposite_ear() {
        let config = render_crossfeed(&crate::crossfeed::NATURAL);
        assert!(config.contains("Copy: L=L+"), "{config}");
        assert!(config.contains("*XR"), "L must receive R's copy: {config}");
        assert!(config.contains("Copy: R=R+"), "{config}");
        assert!(config.contains("*XL"), "R must receive L's copy: {config}");

        // And the copies must be made before they are used.
        let copy_at = config.find("Copy: XL=L XR=R").expect("no virtual channels");
        let use_at = config.find("Copy: L=L+").unwrap();
        assert!(copy_at < use_at, "channels used before they are copied");
    }

    /// Crossfeed adds level, so it must take level back. Without this a bass
    /// heavy, near-mono passage clips -- and bass is exactly what the
    /// crossfeed low-pass lets through.
    #[test]
    fn crossfeed_pays_for_itself_in_headroom() {
        let config = render_crossfeed(&crate::crossfeed::SPEAKERS);
        let preamp = config
            .lines()
            .find(|l| l.starts_with("Preamp:"))
            .expect("crossfeed applied no compensating preamp");
        let db: f32 = preamp
            .trim_start_matches("Preamp:")
            .trim()
            .trim_end_matches(" dB")
            .parse()
            .unwrap();
        assert!(db < -2.0, "compensation was only {db} dB");
    }

    /// Crossfeed must act on the corrected signal, not the raw one.
    #[test]
    fn crossfeed_comes_after_the_correction() {
        let bus = BusCurve::from_composite("Game (TC-HELICON GoXLR)", curve("x", -6.0))
            .with_extras(Extras {
                crossfeed: Some(crate::crossfeed::NATURAL),
                plugin: None,
            });
        let config = render(&[bus]);
        let filter_at = config.find("Filter 1:").expect("no filter");
        let crossfeed_at = config.find("Copy: XL=L").expect("no crossfeed");
        assert!(filter_at < crossfeed_at, "crossfeed ran before the curve");
    }

    /// The plugin runs last, because a compressor should see the finished mix.
    #[test]
    fn the_plugin_runs_at_the_end_of_the_chain() {
        let bus = BusCurve::from_composite("Game (TC-HELICON GoXLR)", curve("x", -6.0))
            .with_extras(Extras {
                crossfeed: Some(crate::crossfeed::NATURAL),
                plugin: Some("Compressor.dll".to_string()),
            });
        let config = render(&[bus]);
        let crossfeed_at = config.find("Copy: XL=L").unwrap();
        let plugin_at = config.find(PLUGIN_DIRECTIVE).expect("no plugin directive");
        assert!(crossfeed_at < plugin_at);
        assert!(config.contains("Compressor.dll"));
    }

    /// Bypass must leave a file that says what it is. A missing file is a
    /// different state to APO and a mystery to anyone reading the directory.
    #[test]
    fn bypass_leaves_an_explanation_rather_than_an_empty_file() {
        let config = render_bypassed();
        assert!(config.starts_with(MARKER));
        assert!(config.to_lowercase().contains("bypassed"));
        assert!(!config.contains("Filter"));
        assert!(!config.contains("Device:"));
    }

    /// The stock APO install ships a preamp and a bass-boosting example, both
    /// of which load before Attune's config and colour every measurement.
    #[test]
    fn interfering_directives_are_recognised() {
        let text = "Preamp: -6 dB\nInclude: example.txt\n\
                    GraphicEQ: 25 0; 40 0\n# a comment\n\
                    # Added by Attune\nInclude: attune.txt\nPreamp: -3 dB\n";

        let mut found = Vec::new();
        let mut reached_attune = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if line.to_ascii_lowercase().contains("include:") && line.contains(CONFIG_FILE) {
                reached_attune = true;
                break;
            }
            found.push(line);
        }

        assert!(reached_attune, "never found Attune's own include");
        assert!(found.contains(&"Preamp: -6 dB"));
        assert!(found.contains(&"Include: example.txt"));
        // Anything after Attune's include is Attune's own and not interference.
        assert!(!found.contains(&"Preamp: -3 dB"));
    }

    /// A flat graphic equaliser is APO's own default and must not be reported
    /// as interference -- warning about the default trains people to ignore
    /// the warning.
    #[test]
    fn a_flat_graphic_eq_is_not_interference() {
        assert!(is_flat_graphic_eq(
            "GraphicEQ: 25 0; 40 0; 63 0; 100 0; 1000 0"
        ));
        assert!(!is_flat_graphic_eq("GraphicEQ: 25 0; 40 6; 63 0"));
        assert!(!is_flat_graphic_eq("GraphicEQ: 25 -3.5; 40 0"));
    }

    /// End to end against the real Equalizer APO parser.
    ///
    /// Ignored because it needs APO installed and it writes to its config
    /// directory. Run it with `cargo test -p attune-eq -- --ignored` after
    /// touching anything in `render_crossfeed`.
    ///
    /// This matters more than it looks. APO silently ignores directives it
    /// does not understand -- no error, no log line, no effect -- so a
    /// rendering mistake here would produce a configuration that loads
    /// cleanly, sounds like nothing happened, and passes every unit test in
    /// this file. The only way to know is to ask APO what it accepted.
    ///
    /// The device name matches no real hardware, so live audio is untouched.
    #[test]
    #[ignore = "needs Equalizer APO installed; writes to its config directory"]
    fn apo_itself_accepts_every_line_of_a_rendered_crossfeed() {
        let Some(install) = detect() else {
            panic!("Equalizer APO is not installed");
        };

        let path = install.attune_config();
        let restore = std::fs::read_to_string(&path).unwrap_or_default();

        let bus = BusCurve {
            endpoint: "AttuneProbe".to_string(),
            connection: None,
            curve: curve("probe", -4.0),
            extras: Extras {
                crossfeed: Some(crate::crossfeed::NATURAL),
                plugin: None,
            },
        };
        std::fs::write(&path, render(&[bus])).unwrap();

        let trace = trace_config(&install, "AttuneProbe", None);
        std::fs::write(&path, restore).unwrap();

        let trace = trace.expect("Benchmark.exe could not be run");

        // Each of these is APO reporting that it accepted a directive. An
        // absent line means the directive was dropped on the floor.
        for expected in [
            "Copying to channel XL from channel L",
            "Adding low-pass filter with frequency 700 Hz",
            "Delaying by 0.3 ms",
            "from channel XR with factor 0.5957",
            "from channel XL with factor 0.5957",
        ] {
            assert!(
                trace.contains(expected),
                "APO did not report accepting {expected:?}.\n{trace}"
            );
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
