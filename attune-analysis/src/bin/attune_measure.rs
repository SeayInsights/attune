//! `attune-measure` -- record the mic and report what it measures.
//!
//! ```text
//! attune-measure --list                  list input devices
//! attune-measure                         record 10s from the GoXLR mic
//! attune-measure --seconds 15            record for longer
//! attune-measure --device "Chat Mic"     pick a device by substring
//! attune-measure --save capture.wav      keep the recording
//! ```
//!
//! Read-only with respect to the device: it changes no settings.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use attune_analysis::capture::{self, list_input_devices};
use attune_analysis::measure::measure;

/// Default device substring. The GoXLR's mic bus as Windows names it.
const DEFAULT_DEVICE: &str = "Chat Mic";

const DEFAULT_SECONDS: u64 = 10;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--list") {
        println!();
        println!("  INPUT DEVICES");
        println!("  -------------");
        let devices = list_input_devices();
        if devices.is_empty() {
            println!("  (none found)");
        }
        for device in devices {
            println!("  {device}");
        }
        println!();
        return ExitCode::SUCCESS;
    }

    let device = arg_value(&args, "--device").unwrap_or_else(|| DEFAULT_DEVICE.to_string());
    let seconds: u64 = arg_value(&args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SECONDS);
    let save = arg_value(&args, "--save").map(PathBuf::from);

    println!();
    println!("  Recording {seconds}s from '{device}'.");
    println!("  Speak normally, at your usual distance and level.");
    println!();

    let capture = match capture::record(&device, Duration::from_secs(seconds)) {
        Ok(capture) => capture,
        Err(e) => {
            eprintln!("  attune-measure: {e}");
            eprintln!();
            eprintln!("  Try --list to see what is available.");
            return ExitCode::FAILURE;
        }
    };

    if let Some(path) = &save {
        match capture::write_wav(&capture, path) {
            Ok(()) => println!("  Saved capture to {}", path.display()),
            Err(e) => eprintln!("  Could not save capture: {e}"),
        }
    }

    let m = measure(&capture.samples, capture.sample_rate);

    println!("  CAPTURE");
    println!("  -------");
    println!("  {:<22} {}", "Device", capture.device_name);
    println!("  {:<22} {} Hz", "Sample rate", m.sample_rate);
    println!("  {:<22} {:.1} s", "Duration", m.duration_secs);
    println!(
        "  {:<22} {} of {}",
        "Speech frames", m.speech_frames, m.total_frames
    );
    println!();

    println!("  LEVELS");
    println!("  ------");
    println!("  {:<22} {:>7.1} dBFS", "Noise floor", m.noise_floor_dbfs);
    println!("  {:<22} {:>7.1} dBFS", "Speech level", m.speech_level_dbfs);
    println!("  {:<22} {:>7.1} dBFS", "Peak", m.peak_dbfs);
    println!(
        "  {:<22} {:>7.1} dB",
        "Signal to noise", m.signal_to_noise_db
    );
    println!("  {:<22} {:>7.1} dB", "Crest factor", m.crest_factor_db);
    if m.clipped_fraction > 0.0 {
        println!(
            "  {:<22} {:>7.3}%  <-- clipping",
            "Clipped samples",
            m.clipped_fraction * 100.0
        );
    }
    println!();

    if !m.has_usable_speech() {
        println!(
            "  Only {} speech frames were found, which is not enough to",
            m.speech_frames
        );
        println!("  judge tone. The level figures above still stand; the spectral");
        println!("  ones below would be noise. Re-run and speak throughout.");
        println!();
        return ExitCode::FAILURE;
    }

    println!("  SPECTRUM  (octave bands, relative to average density)");
    println!("  --------");
    for band in &m.bands {
        let label = if band.centre_hz >= 1000.0 {
            format!("{:.0} kHz", band.centre_hz / 1000.0)
        } else {
            format!("{:.0} Hz", band.centre_hz)
        };
        // A simple bar, so the tilt is visible without reading numbers.
        let bar_len = ((band.level_db + 12.0).clamp(0.0, 24.0) * 1.5) as usize;
        println!(
            "  {:<10} {:>6.1} dB  {}",
            label,
            band.level_db,
            "#".repeat(bar_len)
        );
    }
    println!();

    println!("  These are measurements, not recommendations. Deriving gate and");
    println!("  compressor settings from them is the next work order.");
    println!();

    ExitCode::SUCCESS
}

/// Read `--flag value` out of the argument list.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
