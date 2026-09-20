//! Rule-based analysis of a mic chain.
//!
//! This is the deterministic baseline the tuner builds on. It inspects *settings*
//! only -- it does not listen to anything. Measurement-driven tuning arrives with
//! the analysis crate; this module answers the cheaper question of whether the
//! chain is configured coherently at all.
//!
//! The findings here are heuristics with stated reasoning, not measurements. Each
//! one says why it fired so the operator can disagree with it.

use goxlr_types::CompressorRatio;

use crate::settings::MicChain;

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing, nothing is wrong.
    Info,
    /// Working, but leaving something on the table.
    Warning,
    /// This stage is effectively doing nothing.
    Critical,
}

impl Severity {
    /// Short label for terminal output.
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARN",
            Severity::Critical => "CRIT",
        }
    }
}

/// One observation about the chain.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    /// Which stage of the chain this concerns.
    pub stage: &'static str,
    /// What is wrong, in one line.
    pub summary: String,
    /// Why it matters, and what to do about it.
    pub detail: String,
}

/// The compressor threshold ceiling. At this value nothing ever crosses it.
const COMPRESSOR_DISABLED_THRESHOLD: i8 = 0;

/// Below this, a gate is open to essentially everything including room noise.
const GATE_PERMISSIVE_THRESHOLD_DB: i8 = -50;

/// A dynamic mic below this gain is likely under-driven on most preamps.
const DYNAMIC_GAIN_LOW_DB: u16 = 40;

/// Inspect a chain and return everything worth saying about it.
///
/// An empty result means nothing fired, which is a real answer rather than a
/// failure to look.
pub fn diagnose(chain: &MicChain) -> Vec<Finding> {
    let mut findings = Vec::new();

    // --- Compressor -------------------------------------------------------

    if chain.compressor_threshold_db >= COMPRESSOR_DISABLED_THRESHOLD {
        findings.push(Finding {
            severity: Severity::Critical,
            stage: "Compressor",
            summary: format!(
                "Threshold is {} dB, so the compressor never engages.",
                chain.compressor_threshold_db
            ),
            detail: format!(
                "0 dB is the top of the range: no signal ever crosses it, so the \
                 ratio of {:?} never applies. The chain is running uncompressed, \
                 which means quiet words stay quiet and loud ones stay loud. A \
                 starting point for spoken voice is a threshold somewhere between \
                 -20 and -10 dB -- but the right value depends on how loud you \
                 actually are, which is what measurement settles.",
                chain.compressor_ratio
            ),
        });
    } else if chain.compressor_ratio == CompressorRatio::Ratio1_0 {
        findings.push(Finding {
            severity: Severity::Critical,
            stage: "Compressor",
            summary: "Ratio is 1.0, which is no compression.".to_string(),
            detail: "A 1:1 ratio passes the signal through unchanged no matter \
                 where the threshold sits."
                .to_string(),
        });
    }

    // --- Noise gate -------------------------------------------------------

    if !chain.gate_enabled {
        findings.push(Finding {
            severity: Severity::Critical,
            stage: "Gate",
            summary: "The noise gate is disabled.".to_string(),
            detail: "Everything the mic hears between words goes out: keyboard, \
                 fans, and anything leaking from open-back headphones."
                .to_string(),
        });
    } else if chain.gate_threshold_db <= GATE_PERMISSIVE_THRESHOLD_DB {
        findings.push(Finding {
            severity: Severity::Critical,
            stage: "Gate",
            summary: format!(
                "Threshold is {} dB, low enough that the gate will rarely close.",
                chain.gate_threshold_db
            ),
            detail: "At this threshold almost any sound holds the gate open, so it \
                 is enabled but not doing its job. This matters more with \
                 open-back headphones, which leak your game and music audio into \
                 the mic. The correct value sits just above your room's noise \
                 floor -- measuring that floor is the only way to place it \
                 properly."
                .to_string(),
        });
    }

    if chain.gate_enabled && chain.gate_attenuation < 100 {
        findings.push(Finding {
            severity: Severity::Info,
            stage: "Gate",
            summary: format!(
                "Gate attenuation is {}%, so it ducks rather than mutes.",
                chain.gate_attenuation
            ),
            detail: "Partial attenuation sounds more natural than a hard cut, and \
                 is often the better choice. Flagged only so the setting is a \
                 decision rather than an accident."
                .to_string(),
        });
    }

    // --- Preamp -----------------------------------------------------------

    if chain.mic_type == "Dynamic" && chain.gain_db < DYNAMIC_GAIN_LOW_DB {
        findings.push(Finding {
            severity: Severity::Warning,
            stage: "Preamp",
            summary: format!(
                "Dynamic mic running at {} dB gain, which is low for the type.",
                chain.gain_db
            ),
            detail: "Dynamic mics are passive and quiet; over XLR they typically \
                 want 45-60 dB. If you sound quiet to others, this is the first \
                 place to look. If you are close to the mic and already loud \
                 enough, it is fine as is -- level measurement decides, not this \
                 rule."
                .to_string(),
        });
    }

    findings
}
