//! Live level metering, one meter per output bus.
//!
//! # Why this is worth having
//!
//! Every tuning decision in Attune is made against a number you cannot see:
//! how loud the bus actually is. A correction that peaks the bus is clipping
//! whatever the curve says; a bus sitting 30 dB down is the reason a
//! compressor never engages. Both are invisible in a settings screen and
//! obvious in a meter.
//!
//! # How the signal is obtained
//!
//! WASAPI loopback. Opening a *render* endpoint as an input stream hands back
//! what is being played on it, and cpal does this transparently: it sets
//! `AUDCLNT_STREAMFLAGS_LOOPBACK` when the device it is asked to record from
//! is a render device. So this needs no virtual cable, no driver, and adds
//! nothing to the path anyone is listening to -- it is a tap, not an insert.
//!
//! Loopback is taken **after** the Windows audio engine, so it hears the
//! corrections Equalizer APO applied. That is the right place for a meter: it
//! shows what is leaving for the headphones, which is the thing that can clip.
//!
//! # Peak and RMS, because one number is not enough
//!
//! Peak says whether it is about to clip. RMS says how loud it sounds. Game
//! audio has a huge gap between the two -- an explosion peaks 20 dB above the
//! dialogue that surrounds it -- and that gap is exactly what someone reaching
//! for loudness levelling is reacting to, so both are reported.
//!
//! Peak decays slowly and RMS does not, which is the conventional behaviour
//! and the useful one: a transient you did not see is worth holding on screen.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

/// Anything below this reads as silence rather than as a very small number.
/// -90 dBFS is below the noise floor of any real playback path.
const FLOOR_DBFS: f32 = -90.0;

/// How fast the held peak falls, in dB per second. Slow enough to see a
/// transient you were not watching for, fast enough not to lie about the
/// current level.
const PEAK_DECAY_DB_PER_SEC: f32 = 20.0;

/// How long RMS is averaged over. Roughly the integration time of hearing --
/// short enough to track speech, long enough not to flicker.
const RMS_WINDOW: Duration = Duration::from_millis(300);

/// Meters stop on their own if nobody is looking. The UI polls while the tab
/// is open; when it stops, so does the capture, rather than holding loopback
/// streams open on four endpoints forever.
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// One bus's levels, in dBFS.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Level {
    /// Loudness, averaged over [`RMS_WINDOW`].
    pub rms_dbfs: f32,
    /// Highest sample seen recently, decaying.
    pub peak_dbfs: f32,
    /// Whether a sample hit full scale. Sticky for a moment, because a clip
    /// you did not see is the one that matters.
    pub clipped: bool,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            rms_dbfs: FLOOR_DBFS,
            peak_dbfs: FLOOR_DBFS,
            clipped: false,
        }
    }
}

/// Running state for one bus.
struct BusMeter {
    level: Level,
    /// Sum of squares and count, for the current RMS window.
    square_sum: f64,
    samples: u64,
    window_started: Instant,
    clip_until: Option<Instant>,
    last_update: Instant,
}

impl BusMeter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            level: Level::default(),
            square_sum: 0.0,
            samples: 0,
            window_started: now,
            clip_until: None,
            last_update: now,
        }
    }

    fn feed(&mut self, data: &[f32]) {
        let now = Instant::now();

        let mut block_peak = 0.0_f32;
        for sample in data {
            let magnitude = sample.abs();
            if magnitude > block_peak {
                block_peak = magnitude;
            }
            self.square_sum += (*sample as f64) * (*sample as f64);
        }
        self.samples += data.len() as u64;

        // Decay the held peak by however long it has been, then let this
        // block raise it. Decaying by elapsed time rather than per callback
        // keeps the fall rate honest whatever the buffer size is.
        let elapsed = now.duration_since(self.last_update).as_secs_f32();
        self.level.peak_dbfs =
            (self.level.peak_dbfs - PEAK_DECAY_DB_PER_SEC * elapsed).max(FLOOR_DBFS);
        let block_peak_db = to_dbfs(block_peak);
        if block_peak_db > self.level.peak_dbfs {
            self.level.peak_dbfs = block_peak_db;
        }
        self.last_update = now;

        // Full scale in a float stream means the mix already clipped upstream
        // or is about to on the way out.
        if block_peak >= 0.999 {
            self.clip_until = Some(now + Duration::from_secs(2));
        }
        self.level.clipped = self.clip_until.is_some_and(|until| now < until);

        if now.duration_since(self.window_started) >= RMS_WINDOW && self.samples > 0 {
            let mean = self.square_sum / self.samples as f64;
            self.level.rms_dbfs = to_dbfs((mean.sqrt()) as f32);
            self.square_sum = 0.0;
            self.samples = 0;
            self.window_started = now;
        }
    }
}

fn to_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        return FLOOR_DBFS;
    }
    (20.0 * amplitude.log10()).max(FLOOR_DBFS)
}

/// Shared state between the audio callbacks and whoever is reading.
#[derive(Default)]
struct Shared {
    levels: HashMap<String, Level>,
    /// When a reader last asked. Capture stops when this goes stale.
    last_read: Option<Instant>,
}

/// A running set of meters.
pub struct Meters {
    shared: Arc<Mutex<Shared>>,
    /// Kept so the streams stay alive; dropping this stops the capture.
    _threads: Vec<std::thread::JoinHandle<()>>,
}

/// Errors starting a meter.
#[derive(Debug, thiserror::Error)]
pub enum MeterError {
    #[error(
        "no playback device matching '{0}'. Metering reads the bus through \
         Windows loopback, so the endpoint has to exist and be enabled."
    )]
    NoSuchDevice(String),

    #[error("could not open a loopback stream on {device}: {source}")]
    Stream {
        device: String,
        #[source]
        source: anyhow::Error,
    },
}

impl Meters {
    /// Start metering every named bus.
    ///
    /// A bus that cannot be opened is skipped rather than failing the lot:
    /// someone with three of the four endpoints enabled should still get three
    /// meters. The names that did start are returned.
    pub fn start(buses: &[&str]) -> (Self, Vec<String>, Vec<MeterError>) {
        let shared = Arc::new(Mutex::new(Shared {
            levels: HashMap::new(),
            last_read: Some(Instant::now()),
        }));

        let mut threads = Vec::new();
        let mut started = Vec::new();
        let mut failures = Vec::new();

        for bus in buses {
            match spawn_meter(bus, Arc::clone(&shared)) {
                Ok(handle) => {
                    // Seed the level at the floor. WASAPI loopback delivers no
                    // callbacks at all while an endpoint is idle -- not zeroed
                    // buffers, nothing -- so a bus nobody is playing to would
                    // otherwise have no entry, and a reader could not tell
                    // "silent" from "never started". Measured: opening all
                    // four GoXLR buses with nothing playing produced no data
                    // for any of them.
                    if let Ok(mut shared) = shared.lock() {
                        shared.levels.insert((*bus).to_string(), Level::default());
                    }
                    threads.push(handle);
                    started.push((*bus).to_string());
                }
                Err(e) => failures.push(e),
            }
        }

        (
            Self {
                shared,
                _threads: threads,
            },
            started,
            failures,
        )
    }

    /// Current levels. Reading also marks the meters as wanted, which is what
    /// keeps them running.
    pub fn read(&self) -> HashMap<String, Level> {
        let mut shared = match self.shared.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        shared.last_read = Some(Instant::now());
        shared.levels.clone()
    }

    /// Whether anybody has looked recently. Used to decide when to stop.
    pub fn wanted(&self) -> bool {
        let shared = match self.shared.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        shared
            .last_read
            .is_some_and(|at| at.elapsed() < IDLE_TIMEOUT)
    }
}

/// Open a loopback stream on one render endpoint and keep it alive.
///
/// The stream lives on its own thread because cpal's stream handle is not
/// `Send` on every backend, and because a blocked or slow endpoint should not
/// hold up the others.
fn spawn_meter(
    bus: &str,
    shared: Arc<Mutex<Shared>>,
) -> Result<std::thread::JoinHandle<()>, MeterError> {
    let bus = bus.to_string();
    let device = find_render_device(&bus)?;
    let config = device
        .default_output_config()
        .map_err(|e| MeterError::Stream {
            device: bus.clone(),
            source: e.into(),
        })?;

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let name = bus.clone();

    let handle = std::thread::spawn(move || {
        let mut meter = BusMeter::new();
        let shared_for_callback = Arc::clone(&shared);
        let name_for_callback = name.clone();

        // cpal enables loopback automatically because this is a render
        // device -- there is no flag to pass here.
        let stream = device.build_input_stream(
            &config.config(),
            move |data: &[f32], _| {
                meter.feed(data);
                if let Ok(mut shared) = shared_for_callback.lock() {
                    shared
                        .levels
                        .insert(name_for_callback.clone(), meter.level);
                }
            },
            move |e| log::debug!("meter stream error: {e}"),
            None,
        );

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        if let Err(e) = stream.play() {
            let _ = ready_tx.send(Err(e.to_string()));
            return;
        }
        let _ = ready_tx.send(Ok(()));

        // Hold the stream open until nobody is reading. Without this the
        // stream drops at the end of this closure and the meter dies.
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let stale = match shared.lock() {
                Ok(s) => s.last_read.is_none_or(|at| at.elapsed() >= IDLE_TIMEOUT),
                Err(_) => true,
            };
            if stale {
                break;
            }
        }
    });

    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(message)) => Err(MeterError::Stream {
            device: bus,
            source: anyhow::anyhow!(message),
        }),
        Err(_) => Err(MeterError::Stream {
            device: bus,
            source: anyhow::anyhow!("the device did not start within three seconds"),
        }),
    }
}

/// Find a render endpoint by substring, the same way the rest of Attune
/// resolves a bus name to a device.
fn find_render_device(needle: &str) -> Result<cpal::Device, MeterError> {
    let host = cpal::default_host();
    let needle_lower = needle.to_lowercase();

    host.output_devices()
        .map_err(|_| MeterError::NoSuchDevice(needle.to_string()))?
        .find(|d| {
            d.name()
                .map(|n| n.to_lowercase().contains(&needle_lower))
                .unwrap_or(false)
        })
        .ok_or_else(|| MeterError::NoSuchDevice(needle.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_reads_as_the_floor_not_as_minus_infinity() {
        assert_eq!(to_dbfs(0.0), FLOOR_DBFS);
        assert_eq!(to_dbfs(-0.0), FLOOR_DBFS);
        // And something genuinely tiny is clamped rather than reported as
        // -300 dB, which no meter can draw.
        assert_eq!(to_dbfs(1e-20), FLOOR_DBFS);
    }

    #[test]
    fn full_scale_is_zero_dbfs() {
        assert!((to_dbfs(1.0) - 0.0).abs() < 0.001);
        assert!((to_dbfs(0.5) + 6.02).abs() < 0.01);
    }

    /// RMS and peak must differ on real signals, because the gap between them
    /// is the whole reason both are shown.
    #[test]
    fn a_sine_reads_three_db_below_its_peak() {
        let mut meter = BusMeter::new();
        let samples: Vec<f32> = (0..48_000)
            .map(|i| (i as f32 * 0.1).sin() * 0.5)
            .collect();

        meter.feed(&samples);
        std::thread::sleep(RMS_WINDOW);
        meter.feed(&samples);

        // A sine's RMS is its peak over root two, which is 3.01 dB down.
        let gap = meter.level.peak_dbfs - meter.level.rms_dbfs;
        assert!(
            (gap - 3.01).abs() < 0.5,
            "peak {} rms {} gap {gap}",
            meter.level.peak_dbfs,
            meter.level.rms_dbfs
        );
    }

    /// A clip has to stay visible after the sample that caused it is gone.
    #[test]
    fn a_clip_is_held_long_enough_to_be_seen() {
        let mut meter = BusMeter::new();
        meter.feed(&[0.0, 1.0, 0.0]);
        assert!(meter.level.clipped);

        // Still flagged on the next quiet block.
        meter.feed(&[0.001; 128]);
        assert!(meter.level.clipped, "the clip was forgotten immediately");
    }

    /// Against real hardware. Ignored because it needs a GoXLR present.
    ///
    /// The thing being proved is that cpal will open a *render* endpoint as an
    /// input stream at all -- the whole feature rests on WASAPI loopback being
    /// transparent, and if it is not, everything above is arithmetic on data
    /// that never arrives.
    #[test]
    #[ignore = "needs a GoXLR attached"]
    fn loopback_opens_on_every_goxlr_bus() {
        let buses = ["Game", "Music", "Chat", "System"];
        let (meters, started, failures) = Meters::start(&buses);

        for failure in &failures {
            eprintln!("could not start: {failure}");
        }
        assert!(
            !started.is_empty(),
            "no bus could be metered at all; loopback is not working"
        );

        // Give the callbacks a moment to fire even on a silent bus.
        std::thread::sleep(Duration::from_millis(600));
        let levels = meters.read();

        for bus in &started {
            let level = levels
                .get(bus)
                .unwrap_or_else(|| panic!("{bus} started but reported no level"));
            eprintln!(
                "{bus:<8} rms {:>7.1} dBFS   peak {:>7.1} dBFS",
                level.rms_dbfs, level.peak_dbfs
            );
            assert!(level.rms_dbfs <= 0.01, "{bus} reported above full scale");
            assert!(level.rms_dbfs >= FLOOR_DBFS);
        }
    }

    /// The meter has to read actual signal, not merely open a stream.
    ///
    /// Opening successfully and reporting the floor forever is a failure that
    /// looks like success -- exactly the shape this project keeps running
    /// into -- so this plays pink noise through a bus and insists the meter
    /// notices. Audible for two seconds; ignored by default for that reason.
    #[test]
    #[ignore = "needs a GoXLR attached; plays audible noise for two seconds"]
    fn a_meter_follows_real_audio_on_its_bus() {
        let (meters, started, _) = Meters::start(&["Game"]);
        assert!(started.contains(&"Game".to_string()), "Game did not start");

        let silent = meters.read()["Game"];
        assert!(
            silent.rms_dbfs <= FLOOR_DBFS + 0.01,
            "expected silence before playback, got {} dBFS",
            silent.rms_dbfs
        );

        let noise: Vec<f32> = crate::playback::pink_noise(48_000 * 2, 7)
            .iter()
            .map(|s| s * 0.2)
            .collect();
        let player = std::thread::spawn(move || {
            let _ = crate::playback::play("Game", &noise, 48_000);
        });

        // Sample the meter while the noise is actually playing.
        std::thread::sleep(Duration::from_millis(1200));
        let loud = meters.read()["Game"];
        let _ = player.join();

        eprintln!(
            "silent {:.1} dBFS -> playing {:.1} dBFS (peak {:.1})",
            silent.rms_dbfs, loud.rms_dbfs, loud.peak_dbfs
        );
        assert!(
            loud.rms_dbfs > FLOOR_DBFS + 30.0,
            "the meter did not follow the audio: still {} dBFS",
            loud.rms_dbfs
        );
        assert!(loud.rms_dbfs < 0.0, "reported above full scale");
    }

    /// The peak must fall, or the meter becomes a high-water mark that never
    /// tells you anything again.
    #[test]
    fn the_held_peak_decays() {
        let mut meter = BusMeter::new();
        meter.feed(&[0.9]);
        let after_transient = meter.level.peak_dbfs;

        std::thread::sleep(Duration::from_millis(200));
        meter.feed(&[0.0001]);

        assert!(
            meter.level.peak_dbfs < after_transient,
            "peak stayed at {after_transient}"
        );
    }
}
