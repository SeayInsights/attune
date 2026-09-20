//! `attune-check` -- read the connected GoXLR's mic chain and report on it.
//!
//! Read-only. It changes nothing on the device.
//!
//! Requires the daemon to be running.

use std::process::ExitCode;

use attune_control::client::DaemonClient;
use attune_control::diagnose::{Severity, diagnose};

#[tokio::main]
async fn main() -> ExitCode {
    let client = DaemonClient::default();

    let serial = match client.first_serial().await {
        Ok(serial) => serial,
        Err(e) => {
            eprintln!("attune-check: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mixer = match client.mixer(&serial).await {
        Ok(mixer) => mixer,
        Err(e) => {
            eprintln!("attune-check: {e}");
            return ExitCode::FAILURE;
        }
    };

    let chain = match client.mic_chain(&serial).await {
        Ok(chain) => chain,
        Err(e) => {
            eprintln!("attune-check: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!();
    println!("  DEVICE");
    println!("  ------");
    println!("  {:<22} {:?}", "Model", mixer.hardware.device_type);
    println!("  {:<22} {}", "Serial", serial);
    println!("  {:<22} {}", "Profile", mixer.profile_name);
    println!();

    println!("  MIC CHAIN");
    println!("  ---------");
    println!("  {:<22} {}", "Mic type", chain.mic_type);
    println!("  {:<22} {} dB", "Preamp gain", chain.gain_db);
    println!(
        "  {:<22} {} dB  (attenuation {}%, {})",
        "Gate threshold",
        chain.gate_threshold_db,
        chain.gate_attenuation,
        if chain.gate_enabled {
            "enabled"
        } else {
            "DISABLED"
        }
    );
    println!(
        "  {:<22} {} dB  (ratio {:?}, makeup {} dB)",
        "Compressor threshold",
        chain.compressor_threshold_db,
        chain.compressor_ratio,
        chain.compressor_makeup_db
    );
    println!();

    let findings = diagnose(&chain);

    if findings.is_empty() {
        println!("  No findings. The chain is coherently configured.");
        println!();
        return ExitCode::SUCCESS;
    }

    println!("  FINDINGS ({})", findings.len());
    println!("  ------------");
    for finding in &findings {
        println!();
        println!(
            "  [{}] {} -- {}",
            finding.severity.label(),
            finding.stage,
            finding.summary
        );
        for line in wrap(&finding.detail, 72) {
            println!("         {line}");
        }
    }
    println!();

    // A critical finding means a stage is doing nothing. Exit non-zero so this
    // is usable as a check in a script, not just something to read.
    let worst = findings.iter().map(|f| f.severity).max();
    if worst == Some(Severity::Critical) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Wrap text to a width, breaking on whitespace.
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
