//! What "correct" means, expressed as targets rather than as fixed settings.
//!
//! This is the piece that makes the tuner work for anyone. A recommendation is
//! never "set gain to 55" -- that number only means something for one mic, one
//! voice and one distance. It is always "you measured X, the target is Y, so
//! change by Y minus X". The delta is hardware-independent; the absolute value
//! never was.

/// A tuning goal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// Human-readable name.
    pub name: &'static str,
    /// What it is for.
    pub description: &'static str,

    /// Where representative speech should sit, in dBFS.
    pub speech_level_dbfs: f32,
    /// How close to full scale peaks may come. Peaks land above speech level by
    /// the crest factor, so this is what stops a loud laugh from clipping.
    pub peak_ceiling_dbfs: f32,
    /// How far above the measured noise floor to place the gate. Too small and
    /// the gate chatters on room noise; too large and it clips word endings.
    pub gate_margin_db: f32,
    /// How far below speech level to place the compressor threshold. Larger
    /// means more of the signal gets compressed.
    pub compressor_depth_db: f32,
}

/// Balanced default. Controlled but not obviously processed.
pub const STREAMING: Target = Target {
    name: "streaming",
    description: "Balanced for live voice over game and music beds",
    speech_level_dbfs: -20.0,
    peak_ceiling_dbfs: -6.0,
    gate_margin_db: 8.0,
    compressor_depth_db: 10.0,
};

/// More dynamic range preserved, gentler gating.
pub const PODCAST: Target = Target {
    name: "podcast",
    description: "Preserves more dynamics; gentler gate for conversational pauses",
    speech_level_dbfs: -18.0,
    peak_ceiling_dbfs: -6.0,
    gate_margin_db: 6.0,
    compressor_depth_db: 8.0,
};

/// Tightest control, for noisy rooms and hot mixes.
pub const BROADCAST: Target = Target {
    name: "broadcast",
    description: "Tight and consistent; strongest gate for noisy rooms",
    speech_level_dbfs: -16.0,
    peak_ceiling_dbfs: -6.0,
    gate_margin_db: 12.0,
    compressor_depth_db: 14.0,
};

/// Every target, for listing and lookup.
pub const ALL: &[Target] = &[STREAMING, PODCAST, BROADCAST];

/// Look up a target by name, case-insensitively.
pub fn by_name(name: &str) -> Option<Target> {
    ALL.iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .copied()
}
