//! Built-in voicings, and how they combine with a headphone correction.
//!
//! # Two different things that get confused
//!
//! **Correction** is headphone-specific and measured. A DT 990 Pro has a known
//! response, and undoing it takes a curve derived from measuring that model.
//! Attune does not invent these -- AutoEQ publishes thousands of them from real
//! measurements, and they are imported, not bundled. Shipping a made-up curve
//! labelled "DT 990" would be worse than shipping none.
//!
//! **Voicing** is what you want on top, and it is not headphone-specific. "Cut
//! the bass so footsteps are not masked" is the same instruction on any
//! headphone. These are opinions about a use case, and they are built in.
//!
//! The two compose: correction flattens the headphone, voicing shapes it for a
//! job. Keeping them separate means one headphone measurement serves every
//! voicing, and a new voicing needs no measurements at all.

use crate::curve::{Curve, Filter, FilterKind};

/// A named voicing.
#[derive(Debug, Clone, Copy)]
pub struct Voicing {
    pub name: &'static str,
    pub description: &'static str,
    filters: &'static [Filter],
}

impl Voicing {
    /// This voicing as a curve in its own right.
    pub fn curve(&self) -> Curve {
        Curve {
            name: self.name.to_string(),
            preamp_db: 0.0,
            filters: self.filters.to_vec(),
        }
    }
}

const fn pk(freq_hz: f32, gain_db: f32, q: f32) -> Filter {
    Filter {
        kind: FilterKind::Peaking,
        freq_hz,
        gain_db,
        q,
    }
}

const fn ls(freq_hz: f32, gain_db: f32, q: f32) -> Filter {
    Filter {
        kind: FilterKind::LowShelf,
        freq_hz,
        gain_db,
        q,
    }
}

/// Nothing added. Correction only.
pub const NEUTRAL: Voicing = Voicing {
    name: "neutral",
    description: "Correction only -- no voicing on top",
    filters: &[],
};

/// Unmask quiet detail so it is audible without raising the volume.
///
/// Three moves, in order of how much they matter:
///
/// 1. **Cut the bass shelf.** Loud low frequencies psychoacoustically mask
///    quieter information above them. That upward spread of masking is why
///    footsteps disappear under an explosion, and cutting the shelf is the
///    single largest improvement available.
/// 2. **Tame the upper treble.** Not because detail lives there -- it does not
///    -- but because fatigue there is what stops people turning the volume up.
///    Removing the harshness raises the volume you are *willing* to use.
/// 3. **Lift the detail band gently.** Deliberately modest and broad: a narrow
///    boost here makes everything sound artificial and does not improve
///    localisation, which depends on the whole 2-8 kHz region rather than one
///    frequency in it.
pub const COMPETITIVE: Voicing = Voicing {
    name: "competitive",
    description: "Unmasks quiet detail like footsteps; cuts bass and tames fatigue",
    filters: &[
        ls(200.0, -6.0, 0.7),
        pk(80.0, -3.0, 1.0),
        pk(3000.0, 2.5, 0.8),
        pk(9000.0, -4.0, 2.0),
    ],
};

/// A gentle lift at the extremes, for listening rather than working.
pub const MUSIC: Voicing = Voicing {
    name: "music",
    description: "Slight warmth and air, for listening",
    filters: &[ls(120.0, 2.0, 0.7), pk(12000.0, 1.5, 0.7)],
};

/// Speech intelligibility: less below the voice, more where consonants live.
pub const VOICE: Voicing = Voicing {
    name: "voice",
    description: "Speech clarity for chat and calls",
    filters: &[
        ls(150.0, -4.0, 0.7),
        pk(2500.0, 3.0, 0.9),
        pk(7000.0, -2.0, 2.0),
    ],
};

/// Every built-in voicing.
pub const ALL: &[Voicing] = &[NEUTRAL, COMPETITIVE, MUSIC, VOICE];

/// Look up a voicing by name, case-insensitively.
pub fn by_name(name: &str) -> Option<Voicing> {
    ALL.iter()
        .find(|v| v.name.eq_ignore_ascii_case(name))
        .copied()
}

/// Combine a headphone correction with a voicing.
///
/// Filters are concatenated rather than merged. Merging would mean solving for
/// a single filter set matching the summed response, which is both lossy and
/// pointless: APO applies them in series and the result is the same.
pub fn compose(correction: Option<&Curve>, voicing: Voicing) -> Curve {
    let mut out = match correction {
        Some(c) => c.clone(),
        None => Curve::default(),
    };

    let voicing_curve = voicing.curve();
    out.filters.extend(voicing_curve.filters);

    out.name = match (correction.map(|c| c.name.as_str()), voicing.name) {
        (Some(c), "neutral") if !c.is_empty() => c.to_string(),
        (Some(c), v) if !c.is_empty() => format!("{c} + {v}"),
        (_, v) => v.to_string(),
    };

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_competitive_voicing_cuts_bass_and_does_not_boost_it() {
        let c = COMPETITIVE.curve();
        assert!(c.response_at(60.0) < -3.0, "bass should be cut");
        assert!(c.response_at(120.0) < -2.0);
    }

    /// The claim is that it unmasks detail. That means the detail band must end
    /// up above the bass, not merely lifted in isolation.
    #[test]
    fn the_competitive_voicing_puts_detail_above_bass() {
        let c = COMPETITIVE.curve();
        assert!(
            c.response_at(3000.0) > c.response_at(60.0) + 6.0,
            "detail {:.1} dB vs bass {:.1} dB is not enough separation",
            c.response_at(3000.0),
            c.response_at(60.0)
        );
    }

    #[test]
    fn the_competitive_voicing_tames_the_fatigue_region() {
        let c = COMPETITIVE.curve();
        assert!(c.response_at(9000.0) < 0.0, "9 kHz should be reduced");
    }

    #[test]
    fn neutral_changes_nothing() {
        let c = NEUTRAL.curve();
        assert!(c.filters.is_empty());
        assert_eq!(c.response_at(1000.0), 0.0);
    }

    #[test]
    fn composing_keeps_both_sets_of_filters() {
        let correction = Curve {
            name: "DT 990 Pro".into(),
            preamp_db: -6.0,
            filters: vec![pk(8000.0, -6.0, 3.0)],
        };
        let combined = compose(Some(&correction), COMPETITIVE);

        assert_eq!(combined.filters.len(), 1 + COMPETITIVE.filters.len());
        assert_eq!(combined.preamp_db, -6.0, "the correction's preamp survives");
        assert_eq!(combined.name, "DT 990 Pro + competitive");
    }

    #[test]
    fn a_voicing_works_with_no_correction_at_all() {
        let combined = compose(None, COMPETITIVE);
        assert_eq!(combined.filters.len(), COMPETITIVE.filters.len());
        assert_eq!(combined.name, "competitive");
    }

    #[test]
    fn composing_with_neutral_leaves_the_correction_named_and_unchanged() {
        let correction = Curve {
            name: "DT 990 Pro".into(),
            preamp_db: -6.0,
            filters: vec![pk(8000.0, -6.0, 3.0)],
        };
        let combined = compose(Some(&correction), NEUTRAL);
        assert_eq!(combined.filters.len(), 1);
        assert_eq!(combined.name, "DT 990 Pro");
    }

    #[test]
    fn every_voicing_is_reachable_by_name() {
        for v in ALL {
            assert_eq!(by_name(v.name).map(|x| x.name), Some(v.name));
        }
        assert!(by_name("COMPETITIVE").is_some());
        assert!(by_name("nonsense").is_none());
    }
}
