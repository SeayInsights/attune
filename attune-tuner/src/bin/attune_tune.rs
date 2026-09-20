//! `attune-tune` -- measure the mic, derive settings, optionally apply them.
//!
//! ```text
//! attune-tune                        measure and show what it would change
//! attune-tune --apply                measure, then write the changes
//! attune-tune --target podcast       tune for a different goal
//! attune-tune --targets              list available targets
//! attune-tune --seconds 20           record for longer
//! attune-tune --device "Chat Mic"    pick the input by substring
//! ```
//!
//! Without `--apply` nothing is written. That is the default on purpose: this
//! changes the device you are speaking into.
//!
//! Requires the daemon to be running.

use std::process::ExitCode;
use std::time::Duration;

use attune_analysis::capture;
use attune_analysis::measure::measure;
use attune_control::client::DaemonClient;
use attune_tuner::targets;
use attune_tuner::{apply, derive};

const DEFAULT_DEVICE: &str = "Chat Mic";
const DEFAULT_SECONDS: u64 = 15;
const DEFAULT_TARGET: &str = "streaming";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--targets") {
        println!();
        println!("  TARGETS");
        println!("  -------");
        for t in targets::ALL {
            println!("  {:<12} {}", t.name, t.description);
            println!(
                "  {:<12} speech {:.0} dBFS, peaks under {:.0}, gate {:.0} dB over floor",
                "", t.speech_level_dbfs, t.peak_ceiling_dbfs, t.gate_margin_db
            );
            println!();
        }
        return ExitCode::SUCCESS;
    }

    let device = arg(&args, "--device").unwrap_or_else(|| DEFAULT_DEVICE.to_string());
    let seconds: u64 = arg(&args, "--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SECONDS);
    let target_name = arg(&args, "--target").unwrap_or_else(|| DEFAULT_TARGET.to_string());
    let should_apply = args.iter().any(|a| a == "--apply");

    let Some(target) = targets::by_name(&target_name) else {
        eprintln!("  Unknown target '{target_name}'. Try --targets.");
        return ExitCode::FAILURE;
    };

    // --- Read the device --------------------------------------------------

    let client = DaemonClient::default();

    let serial = match client.first_serial().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  attune-tune: {e}");
            return ExitCode::FAILURE;
        }
    };

    let chain = match client.mic_chain(&serial).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  attune-tune: {e}");
            return ExitCode::FAILURE;
        }
    };

    // --- Measure ----------------------------------------------------------

    println!();
    println!("  Target: {} -- {}", target.name, target.description);
    println!("  Recording {seconds}s from '{device}'. Speak normally.");
    println!();

    let captured = match capture::record(&device, Duration::from_secs(seconds)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  attune-tune: {e}");
            return ExitCode::FAILURE;
        }
    };

    let m = measure(&captured.samples, captured.sample_rate);

    println!(
        "  Measured: speech {:.1} dBFS, floor {:.1}, peak {:.1}, SNR {:.1} dB",
        m.speech_level_dbfs, m.noise_floor_dbfs, m.peak_dbfs, m.signal_to_noise_db
    );
    println!(
        "  ({} of {} frames had speech)",
        m.speech_frames, m.total_frames
    );
    println!();

    // --- Derive -----------------------------------------------------------

    let rec = derive(&m, &chain, target);

    if rec.is_empty() && rec.notes.is_empty() {
        println!("  Nothing to change. The chain already matches the target.");
        println!();
        return ExitCode::SUCCESS;
    }

    if !rec.is_empty() {
        println!("  PROPOSED CHANGES");
        println!("  ----------------");
        for change in &rec.changes {
            println!();
            println!("  {}: {} -> {}", change.setting, change.from, change.to);
            for line in wrap(&change.reason, 70) {
                println!("      {line}");
            }
        }
        println!();
    }

    for note in &rec.notes {
        println!("  NOTE");
        for line in wrap(note, 70) {
            println!("      {line}");
        }
        println!();
    }

    // --- Apply ------------------------------------------------------------

    if !should_apply {
        if !rec.is_empty() {
            println!("  Nothing was written. Re-run with --apply to make these changes.");
            println!();
        }
        return ExitCode::SUCCESS;
    }

    println!("  Applying...");
    let applied = match apply::apply(&client, &serial, &rec).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("  attune-tune: {e}");
            return ExitCode::FAILURE;
        }
    };

    for name in &applied.confirmed {
        println!("    confirmed  {name}");
    }
    for (name, why) in &applied.failed {
        println!("    FAILED     {name}: {why}");
    }
    println!();

    if rec.needs_remeasure {
        println!("  Level has changed, so the earlier measurement no longer describes");
        println!("  this chain. Run attune-tune again to place the gate and compressor");
        println!("  against the corrected signal.");
        println!();
    }

    if applied.all_confirmed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
