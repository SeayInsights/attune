//! Writing a recommendation to the device.
//!
//! Order matters. Gain moves the whole signal, so it goes first; the gate and
//! compressor thresholds are positions relative to that signal. Applying them
//! in the other order would place them against a level that is about to change.

use attune_control::ControlError;
use attune_control::client::DaemonClient;

use crate::derive::Recommendation;

/// What happened when a recommendation was written.
#[derive(Debug, Default)]
pub struct Applied {
    /// Settings that were written and confirmed.
    pub confirmed: Vec<String>,
    /// Settings that failed, with the reason.
    pub failed: Vec<(String, String)>,
}

impl Applied {
    /// Whether every write landed.
    pub fn all_confirmed(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Apply a recommendation, verifying each write.
///
/// A failure does not abort the rest. One setting the device declines should not
/// block three that it would have accepted, and the report says exactly which
/// were which rather than leaving the device in a state nobody can name.
pub async fn apply(
    client: &DaemonClient,
    serial: &str,
    rec: &Recommendation,
) -> Result<Applied, ControlError> {
    let mut result = Applied::default();

    if let Some(gain) = rec.gain_db {
        record(
            &mut result,
            "Preamp gain",
            client.set_mic_gain(serial, gain).await,
        );
    }

    if let Some(threshold) = rec.gate_threshold_db {
        record(
            &mut result,
            "Gate threshold",
            client.set_gate_threshold(serial, threshold).await,
        );
    }

    if let Some(percent) = rec.gate_attenuation {
        record(
            &mut result,
            "Gate attenuation",
            client.set_gate_attenuation(serial, percent).await,
        );
    }

    if let Some(threshold) = rec.compressor_threshold_db {
        record(
            &mut result,
            "Compressor threshold",
            client.set_compressor_threshold(serial, threshold).await,
        );
    }

    if let Some(ratio) = rec.compressor_ratio {
        record(
            &mut result,
            "Compressor ratio",
            client.set_compressor_ratio(serial, ratio).await,
        );
    }

    // EQ last: it shapes the signal the stages above have already levelled, and
    // each band is reported separately so a device that declines one band does
    // not make the other nine look uncertain.
    for (key, gain) in &rec.eq {
        let label = format!("EQ {key:?}");
        record(
            &mut result,
            &label,
            client.set_eq_gain(serial, *key, *gain).await,
        );
    }

    Ok(result)
}

fn record(result: &mut Applied, name: &str, outcome: Result<(), ControlError>) {
    match outcome {
        Ok(()) => result.confirmed.push(name.to_string()),
        Err(e) => result.failed.push((name.to_string(), e.to_string())),
    }
}
