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

    /// The spectral shape a voice should have for this target, as anchor points
    /// of (frequency in Hz, level in dB relative to the average band).
    ///
    /// Anchors rather than one value per band, because the full GoXLR's ten
    /// bands and the Mini's six sit on different centres. Interpolating means
    /// one curve serves both, and any future device, without a table per model.
    ///
    /// **These are an opinion, not physics.** They describe a conventional
    /// broadcast-voice balance: rumble removed, the boxy low-mids eased, and
    /// presence lifted where intelligibility lives. Somebody who disagrees
    /// should edit them, which is why they are here as data rather than buried
    /// in the derivation.
    pub voice_curve: &'static [(f32, f32)],
}

/// Conventional spoken-voice shape. Rolls off rumble, eases the boxy region
/// around 250 Hz, lifts presence at 2-4 kHz, keeps the top honest.
const CURVE_BALANCED: &[(f32, f32)] = &[
    (31.5, -12.0),
    (63.0, -8.0),
    (125.0, -2.0),
    (250.0, -2.0),
    (500.0, 0.0),
    (1000.0, 1.0),
    (2000.0, 3.0),
    (4000.0, 3.0),
    (8000.0, 0.0),
    (16000.0, -4.0),
];

/// Warmer and less forward: keeps more low-mid body, lifts presence less.
const CURVE_WARM: &[(f32, f32)] = &[
    (31.5, -10.0),
    (63.0, -5.0),
    (125.0, 0.0),
    (250.0, 0.0),
    (500.0, 0.0),
    (1000.0, 0.0),
    (2000.0, 1.5),
    (4000.0, 1.5),
    (8000.0, -1.0),
    (16000.0, -5.0),
];

/// Tighter and more forward: more rumble removed, more presence, for cutting
/// through a noisy mix.
const CURVE_FORWARD: &[(f32, f32)] = &[
    (31.5, -14.0),
    (63.0, -10.0),
    (125.0, -4.0),
    (250.0, -3.0),
    (500.0, 0.0),
    (1000.0, 2.0),
    (2000.0, 4.0),
    (4000.0, 4.0),
    (8000.0, 1.0),
    (16000.0, -3.0),
];

impl Target {
    /// The target level for a band centred at `hz`.
    ///
    /// Interpolates logarithmically in frequency, because hearing and octave
    /// bands are logarithmic -- interpolating linearly would put the midpoint
    /// between 1 kHz and 16 kHz at 8.5 kHz instead of 4 kHz.
    pub fn level_at(&self, hz: f32) -> f32 {
        let curve = self.voice_curve;
        if curve.is_empty() {
            return 0.0;
        }
        if hz <= curve[0].0 {
            return curve[0].1;
        }
        if hz >= curve[curve.len() - 1].0 {
            return curve[curve.len() - 1].1;
        }

        for pair in curve.windows(2) {
            let (f0, d0) = pair[0];
            let (f1, d1) = pair[1];
            if hz >= f0 && hz <= f1 {
                let t = (hz.ln() - f0.ln()) / (f1.ln() - f0.ln());
                return d0 + t * (d1 - d0);
            }
        }
        0.0
    }
}

/// Balanced default. Controlled but not obviously processed.
pub const STREAMING: Target = Target {
    name: "streaming",
    description: "Balanced for live voice over game and music beds",
    speech_level_dbfs: -20.0,
    peak_ceiling_dbfs: -6.0,
    gate_margin_db: 8.0,
    compressor_depth_db: 10.0,
    voice_curve: CURVE_BALANCED,
};

/// More dynamic range preserved, gentler gating.
pub const PODCAST: Target = Target {
    name: "podcast",
    description: "Preserves more dynamics; gentler gate for conversational pauses",
    speech_level_dbfs: -18.0,
    peak_ceiling_dbfs: -6.0,
    gate_margin_db: 6.0,
    compressor_depth_db: 8.0,
    voice_curve: CURVE_WARM,
};

/// Tightest control, for noisy rooms and hot mixes.
pub const BROADCAST: Target = Target {
    name: "broadcast",
    description: "Tight and consistent; strongest gate for noisy rooms",
    speech_level_dbfs: -16.0,
    peak_ceiling_dbfs: -6.0,
    gate_margin_db: 12.0,
    compressor_depth_db: 14.0,
    voice_curve: CURVE_FORWARD,
};

/// Every target, for listing and lookup.
pub const ALL: &[Target] = &[STREAMING, PODCAST, BROADCAST];

/// Look up a target by name, case-insensitively.
pub fn by_name(name: &str) -> Option<Target> {
    ALL.iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .copied()
}
