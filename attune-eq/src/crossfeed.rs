//! Crossfeed: making headphones stop putting a hard wall between your ears.
//!
//! # What it is, and what it is not
//!
//! On speakers, your left ear hears the right speaker -- slightly later, and
//! duller, because your head is in the way. Headphones remove that entirely, so
//! anything panned hard sits *inside* one ear. Recordings mixed on speakers get
//! an unnaturally wide, fatiguing presentation, and the effect is worst on open
//! headphones with a bright treble.
//!
//! Crossfeed puts a small amount of each channel into the other, low-passed and
//! delayed, approximating what your head would have done. It is the same idea
//! as Bauer's bs2b and the Linkwitz circuit before it.
//!
//! It is **not** surround virtualisation. It does not place sounds around you
//! and it will not help you hear footsteps behind you -- that is what the
//! spatial formats in [`crate::spatial`] are for. Labelling this "spatial
//! audio" would be the marketing answer and the wrong one: it is a stereo
//! width control with a physical justification.
//!
//! # Why it is three numbers and not a slider
//!
//! Level, cutoff and delay are the three things that actually vary between the
//! published designs, so they are what is exposed. The presets are those
//! designs; the sliders exist because heads differ and so do recordings.

use serde::{Deserialize, Serialize};

/// A crossfeed setting.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Crossfeed {
    /// How far below the direct signal the crossfed signal sits, in dB.
    /// Larger means subtler. Bauer's designs land between 3 and 6 dB.
    pub level_db: f32,
    /// Corner frequency of the low-pass on the crossfed signal, in Hz. Your
    /// head shadows high frequencies far more than low ones, which is why the
    /// crossfeed is low-passed rather than broadband.
    pub cutoff_hz: f32,
    /// Interaural time difference, in microseconds. The extra distance to the
    /// far ear is roughly 20 cm, which at the speed of sound is about 300 us.
    pub delay_us: f32,
}

/// The widest setting: a hint of crossfeed, mostly for keeping the stereo image
/// intact on material that was mixed for headphones anyway.
pub const SUBTLE: Crossfeed = Crossfeed {
    level_db: 6.0,
    cutoff_hz: 700.0,
    delay_us: 260.0,
};

/// Bauer's default. The one to start from.
pub const NATURAL: Crossfeed = Crossfeed {
    level_db: 4.5,
    cutoff_hz: 700.0,
    delay_us: 300.0,
};

/// Strongest: closest to listening on speakers, at the cost of some width.
/// Good for old recordings with hard-panned instruments.
pub const SPEAKERS: Crossfeed = Crossfeed {
    level_db: 3.0,
    cutoff_hz: 650.0,
    delay_us: 340.0,
};

/// The named presets, in the order to show them.
pub const PRESETS: &[(&str, Crossfeed, &str)] = &[
    (
        "subtle",
        SUBTLE,
        "A hint of it. Keeps the stereo image wide.",
    ),
    (
        "natural",
        NATURAL,
        "Bauer's default, and the one to start from.",
    ),
    (
        "speakers",
        SPEAKERS,
        "Closest to listening on speakers. Best on older, hard-panned recordings.",
    ),
];

/// Sane bounds. Outside these it stops being crossfeed and starts being a
/// mistake: too much level collapses the image to mono, and a cutoff up in the
/// presence region smears voices.
pub const LEVEL_RANGE_DB: (f32, f32) = (1.0, 12.0);
pub const CUTOFF_RANGE_HZ: (f32, f32) = (300.0, 2000.0);
pub const DELAY_RANGE_US: (f32, f32) = (0.0, 600.0);

impl Default for Crossfeed {
    fn default() -> Self {
        NATURAL
    }
}

impl Crossfeed {
    /// Clamped to the usable range.
    pub fn sane(&self) -> Self {
        Self {
            level_db: self.level_db.clamp(LEVEL_RANGE_DB.0, LEVEL_RANGE_DB.1),
            cutoff_hz: self.cutoff_hz.clamp(CUTOFF_RANGE_HZ.0, CUTOFF_RANGE_HZ.1),
            delay_us: self.delay_us.clamp(DELAY_RANGE_US.0, DELAY_RANGE_US.1),
        }
    }

    /// The linear factor the opposite channel is mixed in at.
    pub fn mix_factor(&self) -> f32 {
        10f32.powf(-self.sane().level_db / 20.0)
    }

    /// The gain reduction needed so the sum cannot clip, in dB (negative).
    ///
    /// Worst case is a mono signal, where both channels are identical and the
    /// output becomes `(1 + g)` times the input. Correlated content is not an
    /// edge case here -- bass is usually near-mono in a real mix, and bass is
    /// exactly what passes the crossfeed low-pass.
    pub fn headroom_db(&self) -> f32 {
        -20.0 * (1.0 + self.mix_factor()).log10()
    }

    /// The delay in milliseconds, which is the unit APO's `Delay:` takes.
    pub fn delay_ms(&self) -> f32 {
        self.sane().delay_us / 1000.0
    }

    /// Which preset this matches exactly, if any.
    pub fn preset_name(&self) -> Option<&'static str> {
        PRESETS
            .iter()
            .find(|(_, preset, _)| preset == self)
            .map(|(name, _, _)| *name)
    }
}

/// Look a preset up by name.
pub fn preset(name: &str) -> Option<Crossfeed> {
    PRESETS
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, c, _)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Larger level_db means *less* crossfeed. Getting this backwards would
    /// invert every control in the UI, and it reads ambiguously either way, so
    /// it is pinned here.
    #[test]
    fn a_bigger_level_means_less_crossfeed() {
        let subtle = SUBTLE.mix_factor();
        let strong = SPEAKERS.mix_factor();
        assert!(
            subtle < strong,
            "6 dB should mix in less than 3 dB: {subtle} vs {strong}"
        );
    }

    /// The published figure for Bauer's default is about 4.5 dB, which is a
    /// mix factor near 0.6.
    #[test]
    fn the_default_matches_the_published_design() {
        let g = NATURAL.mix_factor();
        assert!((g - 0.596).abs() < 0.005, "mix factor was {g}");
    }

    /// Mono content through crossfeed is the clipping case, and it is not an
    /// edge case: bass is usually near-mono and bass is what gets crossfed.
    #[test]
    fn headroom_stops_mono_content_clipping() {
        for c in [SUBTLE, NATURAL, SPEAKERS] {
            let peak = (1.0 + c.mix_factor()) * 10f32.powf(c.headroom_db() / 20.0);
            assert!(
                peak <= 1.0001,
                "{:?} peaks at {peak} on mono content",
                c.preset_name()
            );
        }
    }

    /// Headroom is a cost, not a free win. More crossfeed must cost more level.
    #[test]
    fn more_crossfeed_costs_more_headroom() {
        assert!(SPEAKERS.headroom_db() < SUBTLE.headroom_db());
    }

    #[test]
    fn values_outside_the_usable_range_are_clamped() {
        let silly = Crossfeed {
            level_db: -40.0,
            cutoff_hz: 19_000.0,
            delay_us: 9_000.0,
        };
        let sane = silly.sane();
        assert_eq!(sane.level_db, LEVEL_RANGE_DB.0);
        assert_eq!(sane.cutoff_hz, CUTOFF_RANGE_HZ.1);
        assert_eq!(sane.delay_us, DELAY_RANGE_US.1);

        // And the clamp has to actually protect the mix: an unclamped
        // level of -40 dB would mix the opposite channel in at 100x.
        assert!(silly.mix_factor() < 1.0);
    }

    #[test]
    fn every_preset_is_reachable_by_name() {
        for (name, expected, description) in PRESETS {
            assert_eq!(preset(name), Some(*expected));
            assert!(!description.is_empty(), "{name} has no description");
            assert_eq!(expected.preset_name(), Some(*name));
        }
        assert_eq!(preset("nonsense"), None);
    }
}
