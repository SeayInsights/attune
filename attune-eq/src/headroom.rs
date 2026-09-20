//! Headroom management for correction curves.
//!
//! # What this actually buys, stated carefully
//!
//! A correction curve that boosts anywhere needs headroom, or the boosted band
//! clips. Equalizer APO applies its preamp before the filters, so lowering the
//! preamp by the curve's peak boost guarantees the summed response never exceeds
//! unity. That removes **digital** clipping completely, and it is the real and
//! unambiguous benefit here.
//!
//! # What it does not buy
//!
//! It is tempting to say a cut-only curve demands less from a weak headphone
//! amplifier. At matched loudness that is not true. If a curve boosts bass by
//! 6 dB, and the alternative cuts everything else by 6 dB and you then turn the
//! volume up 6 dB to compensate, the amplifier delivers the same voltage at the
//! bass frequencies either way. The physics does not care which end of the
//! chain the ratio came from.
//!
//! The honest amplifier benefit is narrower: if you **leave the volume where it
//! was**, a cut-only curve produces a lower peak output voltage than a boosting
//! one. You listen slightly quieter, and in exchange the amplifier is further
//! from its limit. On a marginal pairing -- a high-impedance headphone on a
//! modest output -- that trade can be worth making. It is a trade, not a free
//! win, and the caller should be told which one they are choosing.

use crate::curve::Curve;

/// A curve rewritten so its summed response never exceeds unity gain.
///
/// The filter shapes are untouched: only the preamp moves. Rewriting individual
/// filter gains would change the curve's shape, which is the one thing a
/// correction curve must not lose.
#[derive(Debug, Clone)]
pub struct Managed {
    /// The curve, with its preamp adjusted.
    pub curve: Curve,
    /// How much the preamp was lowered, in dB.
    pub headroom_applied_db: f32,
    /// The peak boost the original curve asked for, in dB.
    pub original_peak_db: f32,
}

impl Managed {
    /// Whether anything had to change.
    pub fn is_unchanged(&self) -> bool {
        self.headroom_applied_db.abs() < 0.01
    }

    /// One line for the operator explaining the trade that was made.
    pub fn explain(&self) -> String {
        if self.is_unchanged() {
            return "This curve only cuts, so it needs no headroom and nothing \
                    was changed."
                .to_string();
        }
        format!(
            "This curve boosts by up to {:.1} dB, which would clip. The preamp \
             is lowered {:.1} dB so the total never exceeds unity -- the tonal \
             balance is identical, and the whole output is {:.1} dB quieter. \
             Turn your volume up to compensate; the amplifier then does exactly \
             the work it would have done anyway.",
            self.original_peak_db, self.headroom_applied_db, self.headroom_applied_db
        )
    }
}

/// Give a curve enough headroom that it cannot clip.
///
/// `extra_db` is an additional safety margin beyond the measured peak, for
/// curves whose true biquad response slightly exceeds this crate's approximation
/// of it.
pub fn manage(curve: &Curve, extra_db: f32) -> Managed {
    let peak = curve.peak_gain_db();

    if peak <= 0.0 {
        return Managed {
            curve: curve.clone(),
            headroom_applied_db: 0.0,
            original_peak_db: peak,
        };
    }

    let reduction = peak + extra_db;
    let mut managed = curve.clone();
    managed.preamp_db -= reduction;

    Managed {
        curve: managed,
        headroom_applied_db: reduction,
        original_peak_db: peak,
    }
}

/// The default safety margin, in dB.
pub const DEFAULT_MARGIN_DB: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Filter, FilterKind};

    fn boosting() -> Curve {
        Curve {
            name: "boosting".into(),
            preamp_db: 0.0,
            filters: vec![Filter {
                kind: FilterKind::LowShelf,
                freq_hz: 105.0,
                gain_db: 6.0,
                q: 0.7,
            }],
        }
    }

    fn cutting() -> Curve {
        Curve {
            name: "cutting".into(),
            preamp_db: 0.0,
            filters: vec![Filter {
                kind: FilterKind::Peaking,
                freq_hz: 8000.0,
                gain_db: -6.0,
                q: 2.0,
            }],
        }
    }

    #[test]
    fn a_boosting_curve_gets_headroom() {
        let m = manage(&boosting(), DEFAULT_MARGIN_DB);
        assert!(m.headroom_applied_db > 5.0, "{}", m.headroom_applied_db);
        assert!(m.curve.preamp_db < 0.0);
    }

    #[test]
    fn a_cut_only_curve_is_left_alone() {
        let m = manage(&cutting(), DEFAULT_MARGIN_DB);
        assert!(m.is_unchanged());
        assert_eq!(m.curve.preamp_db, 0.0);
    }

    /// The point of the exercise: after management, no frequency exceeds unity.
    #[test]
    fn a_managed_curve_never_exceeds_unity_anywhere() {
        let m = manage(&boosting(), DEFAULT_MARGIN_DB);
        let mut hz = 20.0_f32;
        while hz < 20_000.0 {
            let total = m.curve.response_at(hz);
            assert!(total <= 0.01, "{total:.2} dB at {hz:.0} Hz exceeds unity");
            hz *= crate::curve::TWELFTH_OCTAVE;
        }
    }

    /// The shape is the correction. Only the preamp may move.
    #[test]
    fn management_shifts_level_without_changing_shape() {
        let original = boosting();
        let m = manage(&original, 0.0);

        let mut hz = 20.0_f32;
        while hz < 20_000.0 {
            let before = original.response_at(hz);
            let after = m.curve.response_at(hz);
            assert!(
                (before - after - m.headroom_applied_db).abs() < 0.001,
                "shape changed at {hz:.0} Hz"
            );
            hz *= crate::curve::TWELFTH_OCTAVE;
        }
    }

    #[test]
    fn the_explanation_does_not_claim_a_free_win() {
        let m = manage(&boosting(), DEFAULT_MARGIN_DB);
        let text = m.explain();
        assert!(text.contains("quieter"), "the trade must be stated: {text}");
    }
}
