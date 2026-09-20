//! Turning a measurement into settings.
//!
//! # Why this converges instead of solving in one shot
//!
//! The GoXLR exposes its mic only after the onboard DSP, so a measurement
//! describes the signal *as currently processed*. There is no way to compute the
//! final answer from one reading.
//!
//! More importantly, the stages are not independent. Gate and compressor
//! thresholds are positions relative to the signal's level -- so deriving them
//! from a signal that is 25 dB below target produces numbers that are precisely
//! wrong. Level has to be right first, and then the rest can be placed against
//! it.
//!
//! So this deliberately refuses to recommend gate or compressor settings while
//! level is out of range. It fixes level, asks for a re-measure, and only then
//! places the rest. Emitting all four numbers at once would look more helpful
//! and be worse.

use attune_analysis::Measurement;
use attune_control::settings::MicChain;
use goxlr_types::{CompressorRatio, MicrophoneType};

use crate::targets::Target;

/// Device limits for the GoXLR preamp, in dB.
const GAIN_MIN_DB: i32 = 0;
const GAIN_MAX_DB: i32 = 72;

/// Device limits for the gate threshold, in dB.
const GATE_MIN_DB: i32 = -59;
const GATE_MAX_DB: i32 = 0;

/// Device limits for the compressor threshold, in dB.
const COMP_MIN_DB: i32 = -40;
const COMP_MAX_DB: i32 = 0;

/// How far speech level may sit from target before level is the only concern.
const LEVEL_TOLERANCE_DB: f32 = 4.0;

/// Largest gain change allowed in one pass.
///
/// A correction is only as good as the measurement behind it, and a measurement
/// can be wrong in ways no guard catches. Bounding the step means a bad reading
/// costs a modest, obvious move that the next measurement corrects, instead of
/// pinning the preamp to a rail in one shot. Converging in several passes is
/// slower and much harder to get badly wrong.
const MAX_GAIN_STEP_DB: f32 = 12.0;

/// Below this signal-to-noise ratio a gate cannot be placed usefully: there is
/// not enough separation between voice and room for any threshold to catch one
/// and not the other.
const MIN_SNR_FOR_GATING_DB: f32 = 15.0;

/// One proposed change.
#[derive(Debug, Clone)]
pub struct Change {
    /// Which setting.
    pub setting: &'static str,
    /// Current value, formatted.
    pub from: String,
    /// Proposed value, formatted.
    pub to: String,
    /// Why, in the operator's terms.
    pub reason: String,
}

/// What the tuner concluded.
#[derive(Debug, Clone)]
pub struct Recommendation {
    /// Changes to apply, in the order they should be applied.
    pub changes: Vec<Change>,
    /// Concrete values, for the apply path.
    pub gain_db: Option<u16>,
    pub gate_threshold_db: Option<i8>,
    pub compressor_threshold_db: Option<i8>,
    pub compressor_ratio: Option<CompressorRatio>,
    /// True when level had to be corrected first and a re-measure is needed
    /// before the remaining stages can be placed.
    pub needs_remeasure: bool,
    /// Things the operator should know that are not settings changes.
    pub notes: Vec<String>,
}

impl Recommendation {
    /// Whether anything at all would change.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Pick the ratio closest to a desired numeric compression ratio.
fn nearest_ratio(desired: f32) -> CompressorRatio {
    const TABLE: &[(f32, CompressorRatio)] = &[
        (1.0, CompressorRatio::Ratio1_0),
        (1.1, CompressorRatio::Ratio1_1),
        (1.2, CompressorRatio::Ratio1_2),
        (1.4, CompressorRatio::Ratio1_4),
        (1.6, CompressorRatio::Ratio1_6),
        (1.8, CompressorRatio::Ratio1_8),
        (2.0, CompressorRatio::Ratio2_0),
        (2.5, CompressorRatio::Ratio2_5),
        (3.2, CompressorRatio::Ratio3_2),
        (4.0, CompressorRatio::Ratio4_0),
        (5.6, CompressorRatio::Ratio5_6),
        (8.0, CompressorRatio::Ratio8_0),
        (16.0, CompressorRatio::Ratio16_0),
        (32.0, CompressorRatio::Ratio32_0),
        (64.0, CompressorRatio::Ratio64_0),
    ];

    TABLE
        .iter()
        .min_by(|a, b| {
            (a.0 - desired)
                .abs()
                .partial_cmp(&(b.0 - desired).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, ratio)| *ratio)
        .unwrap_or(CompressorRatio::Ratio2_0)
}

/// Derive settings for a target from a measurement of the current chain.
pub fn derive(m: &Measurement, chain: &MicChain, target: Target) -> Recommendation {
    let mut changes = Vec::new();
    let mut notes = Vec::new();

    let mut gain_db = None;
    let mut gate_threshold_db = None;
    let mut compressor_threshold_db = None;
    let mut compressor_ratio = None;

    let usability = m.usability();
    if usability != attune_analysis::measure::Usability::Usable {
        notes.push(usability.explain());
        return Recommendation {
            changes,
            gain_db,
            gate_threshold_db,
            compressor_threshold_db,
            compressor_ratio,
            needs_remeasure: true,
            notes,
        };
    }

    // --- Stage 1: level -----------------------------------------------------
    //
    // This is the only figure computed as a delta from measurement, which is
    // what makes it work regardless of mic, voice or distance.

    let level_error = target.speech_level_dbfs - m.speech_level_dbfs;

    // Do not drive peaks past the ceiling chasing an average, and never move
    // further than one bounded step regardless of what the measurement claims.
    let headroom_limit = target.peak_ceiling_dbfs - m.peak_dbfs;
    let applied_delta = level_error
        .min(headroom_limit)
        .clamp(-MAX_GAIN_STEP_DB, MAX_GAIN_STEP_DB);

    let level_off = level_error.abs() > LEVEL_TOLERANCE_DB;

    if level_off {
        let proposed = (chain.gain_db as i32 + applied_delta.round() as i32)
            .clamp(GAIN_MIN_DB, GAIN_MAX_DB) as u16;

        if proposed != chain.gain_db {
            let direction = if applied_delta > 0.0 { "quiet" } else { "loud" };
            let mut reason = format!(
                "Speech measured {:.1} dBFS against a target of {:.1}, so the \
                 signal is {:.0} dB too {}. This is a measured delta, not a \
                 preset: it accounts for your mic, your voice and your distance \
                 without knowing any of them.",
                m.speech_level_dbfs,
                target.speech_level_dbfs,
                level_error.abs(),
                direction
            );

            if applied_delta.abs() >= MAX_GAIN_STEP_DB && level_error.abs() > MAX_GAIN_STEP_DB {
                reason.push_str(&format!(
                    " Moving {MAX_GAIN_STEP_DB:.0} dB this pass rather than the \
                     full {:.0}, so a bad measurement cannot slam the preamp to \
                     a rail. Re-measure and run again to converge.",
                    level_error.abs()
                ));
            } else if applied_delta < level_error {
                reason.push_str(&format!(
                    " Limited to {applied_delta:.0} dB to keep peaks under \
                     {:.0} dBFS.",
                    target.peak_ceiling_dbfs
                ));
            }

            // Whether the *full* correction is reachable, not just this step.
            // Bounded stepping means the ceiling is not hit in one pass, so
            // checking the proposed value would stay silent until several passes
            // in -- by which point the operator has been told nothing useful.
            if chain.gain_db as f32 + level_error > GAIN_MAX_DB as f32 {
                notes.push(format!(
                    "Even at the device maximum of {GAIN_MAX_DB} dB this signal \
                     stays below target -- it needs about {:.0} dB and the \
                     preamp cannot supply it. The rest has to come from physics: \
                     move closer to the mic, speak up, or check the mic type \
                     matches how the mic is actually connected.",
                    chain.gain_db as f32 + level_error
                ));
            }

            changes.push(Change {
                setting: "Preamp gain",
                from: format!("{} dB", chain.gain_db),
                to: format!("{proposed} dB"),
                reason,
            });
            gain_db = Some(proposed);
        }
    }

    // --- Stage 2 and 3: only once level is right ---------------------------

    if level_off {
        notes.push(
            "Gate and compressor are not being set yet. Both are positions \
             relative to the signal's level, so placing them against an \
             out-of-range signal would put them precisely in the wrong spot. \
             Re-measure after applying the gain change and run again."
                .to_string(),
        );

        return Recommendation {
            changes,
            gain_db,
            gate_threshold_db,
            compressor_threshold_db,
            compressor_ratio,
            needs_remeasure: true,
            notes,
        };
    }

    // Gate: sit above the measured floor by the target's margin.
    if m.signal_to_noise_db < MIN_SNR_FOR_GATING_DB {
        notes.push(format!(
            "Signal-to-noise is only {:.0} dB. There is not enough separation \
             between your voice and the room for a gate to catch one without \
             the other. Reduce room noise or get closer to the mic before \
             gating.",
            m.signal_to_noise_db
        ));
    } else {
        let proposed = ((m.noise_floor_dbfs + target.gate_margin_db).round() as i32)
            .clamp(GATE_MIN_DB, GATE_MAX_DB) as i8;

        if proposed != chain.gate_threshold_db {
            changes.push(Change {
                setting: "Gate threshold",
                from: format!("{} dB", chain.gate_threshold_db),
                to: format!("{proposed} dB"),
                reason: format!(
                    "Noise floor measured {:.1} dBFS. Placing the gate {:.0} dB \
                     above it closes on the room without clipping the ends of \
                     words. You have {:.0} dB of separation to work with.",
                    m.noise_floor_dbfs, target.gate_margin_db, m.signal_to_noise_db
                ),
            });
            gate_threshold_db = Some(proposed);
        }
    }

    // Compressor: threshold below speech level, ratio from measured dynamics.
    let proposed_threshold = ((target.speech_level_dbfs - target.compressor_depth_db).round()
        as i32)
        .clamp(COMP_MIN_DB, COMP_MAX_DB) as i8;

    if proposed_threshold != chain.compressor_threshold_db {
        changes.push(Change {
            setting: "Compressor threshold",
            from: format!("{} dB", chain.compressor_threshold_db),
            to: format!("{proposed_threshold} dB"),
            reason: format!(
                "Sits {:.0} dB below the {:.0} dBFS speech target, so ordinary \
                 speech passes and only the louder moments are controlled. Your \
                 previous threshold of {} dB meant the compressor {}.",
                target.compressor_depth_db,
                target.speech_level_dbfs,
                chain.compressor_threshold_db,
                if chain.compressor_threshold_db >= 0 {
                    "never engaged at all"
                } else {
                    "engaged at a different point"
                }
            ),
        });
        compressor_threshold_db = Some(proposed_threshold);
    }

    // Crest factor says how much taming the dynamics actually need.
    let desired_ratio = (m.crest_factor_db / 6.0).clamp(1.5, 8.0);
    let proposed_ratio = nearest_ratio(desired_ratio);

    if proposed_ratio != chain.compressor_ratio {
        changes.push(Change {
            setting: "Compressor ratio",
            from: format!("{:?}", chain.compressor_ratio),
            to: format!("{proposed_ratio:?}"),
            reason: format!(
                "Crest factor measured {:.1} dB -- the gap between your peaks \
                 and your normal speech. That maps to roughly {:.1}:1 to bring \
                 peaks under control without flattening the voice.",
                m.crest_factor_db, desired_ratio
            ),
        });
        compressor_ratio = Some(proposed_ratio);
    }

    if m.clipped_fraction > 0.0 {
        notes.push(format!(
            "{:.3}% of samples were clipped. Clipping is distortion that no \
             later stage can undo -- reduce gain until it stops.",
            m.clipped_fraction * 100.0
        ));
    }

    if chain.mic_type == MicrophoneType::Jack {
        notes.push(
            "Mic type is Jack (3.5 mm). If your microphone is on the XLR input, \
             set the type to Dynamic or Condenser -- gain is stored per type, so \
             the wrong one tunes a preamp you are not using."
                .to_string(),
        );
    }

    Recommendation {
        changes,
        gain_db,
        gate_threshold_db,
        compressor_threshold_db,
        compressor_ratio,
        needs_remeasure: false,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::STREAMING;

    /// A measurement with sane defaults, overridden per test.
    fn measurement(speech: f32, floor: f32, peak: f32) -> Measurement {
        Measurement {
            duration_secs: 15.0,
            sample_rate: 48_000,
            noise_floor_dbfs: floor,
            speech_level_dbfs: speech,
            peak_dbfs: peak,
            signal_to_noise_db: speech - floor,
            crest_factor_db: peak - speech,
            clipped_fraction: 0.0,
            low_mid_db: -12.0,
            presence_db: -20.0,
            sibilance_db: -26.0,
            speech_frames: 400,
            total_frames: 750,
        }
    }

    fn chain(gain: u16) -> MicChain {
        MicChain {
            mic_type: MicrophoneType::Dynamic,
            gain_db: gain,
            gate_threshold_db: -30,
            gate_attenuation: 100,
            gate_enabled: true,
            compressor_threshold_db: -10,
            compressor_ratio: CompressorRatio::Ratio2_0,
            compressor_makeup_db: 0,
        }
    }

    #[test]
    fn a_quiet_signal_asks_for_more_gain() {
        // 26 dB below the -20 target.
        let m = measurement(-46.0, -71.0, -32.0);
        let rec = derive(&m, &chain(30), STREAMING);
        let gain = rec.gain_db.expect("should recommend gain");
        assert!(gain > 30, "expected gain above 30, got {gain}");
    }

    #[test]
    fn a_loud_signal_asks_for_less_gain() {
        let m = measurement(-8.0, -60.0, -2.0);
        let rec = derive(&m, &chain(50), STREAMING);
        let gain = rec.gain_db.expect("should recommend gain");
        assert!(gain < 50, "expected gain below 50, got {gain}");
    }

    /// The property that makes this work on hardware the author has never seen:
    /// the same measured error produces the same correction from any starting
    /// gain, because it is a delta rather than a preset.
    #[test]
    fn the_correction_is_a_delta_not_a_preset() {
        let m = measurement(-40.0, -70.0, -26.0);

        let from_20 = derive(&m, &chain(20), STREAMING).gain_db.unwrap();
        let from_40 = derive(&m, &chain(40), STREAMING).gain_db.unwrap();

        assert_eq!(
            from_40 - from_20,
            20,
            "same measured error should move both starting points by the same amount"
        );
    }

    #[test]
    fn gate_and_compressor_are_withheld_while_level_is_wrong() {
        let m = measurement(-46.0, -71.0, -32.0);
        let rec = derive(&m, &chain(30), STREAMING);

        assert!(rec.needs_remeasure);
        assert!(
            rec.gate_threshold_db.is_none(),
            "gate must not be placed against an out-of-range signal"
        );
        assert!(rec.compressor_threshold_db.is_none());
    }

    #[test]
    fn gate_and_compressor_are_set_once_level_is_right() {
        // On target, with healthy separation.
        let m = measurement(-20.0, -70.0, -6.0);
        let rec = derive(&m, &chain(50), STREAMING);

        assert!(!rec.needs_remeasure);
        assert!(rec.gate_threshold_db.is_some());
        assert!(rec.compressor_threshold_db.is_some());
    }

    #[test]
    fn gate_is_withheld_when_there_is_too_little_separation() {
        // On level, but the room is nearly as loud as the voice.
        let m = measurement(-20.0, -28.0, -6.0);
        let rec = derive(&m, &chain(50), STREAMING);

        assert!(
            rec.gate_threshold_db.is_none(),
            "a gate cannot be placed with only 8 dB of separation"
        );
        assert!(
            rec.notes.iter().any(|n| n.contains("separation")),
            "the operator should be told why"
        );
    }

    #[test]
    fn gain_never_exceeds_the_device_range() {
        // Quiet but plausible, from a gain already near the top.
        let m = measurement(-68.0, -90.0, -60.0);
        let rec = derive(&m, &chain(70), STREAMING);

        let gain = rec.gain_db.expect("should still recommend a change");
        assert!(gain <= GAIN_MAX_DB as u16, "gain {gain} exceeds device max");
    }

    #[test]
    fn hitting_the_gain_ceiling_tells_the_operator_what_to_do_physically() {
        // Needs 48 dB more from a start of 40: past the 72 dB ceiling.
        let m = measurement(-68.0, -90.0, -60.0);
        let rec = derive(&m, &chain(40), STREAMING);

        assert!(
            rec.notes.iter().any(|n| n.contains("closer")),
            "when gain runs out, the remaining fix is physical and should be said"
        );
    }

    /// Regression: this exact capture pinned a live preamp to its maximum.
    ///
    /// 40 speech frames cleared a count-only guard of 20, and a 65 dB
    /// correction was derived from what was effectively a recording of a room.
    #[test]
    fn a_capture_that_is_mostly_silence_is_rejected() {
        let mut m = measurement(-84.9, -93.5, -62.7);
        m.speech_frames = 40;
        m.total_frames = 750;

        let rec = derive(&m, &chain(30), STREAMING);

        assert!(
            rec.gain_db.is_none(),
            "must not derive gain from a capture that is 5% speech"
        );
        assert!(rec.is_empty());
        assert!(rec.needs_remeasure);
    }

    /// No microphone carrying a human voice measures this quietly.
    #[test]
    fn an_implausibly_quiet_capture_is_rejected_even_with_many_frames() {
        let mut m = measurement(-85.0, -95.0, -70.0);
        m.speech_frames = 600;
        m.total_frames = 750;

        let rec = derive(&m, &chain(30), STREAMING);
        assert!(rec.gain_db.is_none());
        assert!(
            rec.notes.iter().any(|n| n.contains("recording of a room")),
            "should name what actually happened"
        );
    }

    /// The safety property: one pass cannot make a large move, however wrong
    /// the measurement is.
    #[test]
    fn gain_moves_by_at_most_one_bounded_step() {
        let m = measurement(-68.0, -90.0, -60.0);
        let before = 30u16;
        let after = derive(&m, &chain(before), STREAMING).gain_db.unwrap();

        let moved = (after as i32 - before as i32).unsigned_abs();
        assert!(
            moved <= MAX_GAIN_STEP_DB as u32,
            "moved {moved} dB in one pass, cap is {MAX_GAIN_STEP_DB}"
        );
    }

    #[test]
    fn a_signal_already_on_target_proposes_no_gain_change() {
        let m = measurement(-20.0, -70.0, -6.0);
        let rec = derive(&m, &chain(50), STREAMING);
        assert!(rec.gain_db.is_none());
    }

    #[test]
    fn nothing_is_derived_from_a_capture_without_speech() {
        let mut m = measurement(-46.0, -71.0, -32.0);
        m.speech_frames = 2;

        let rec = derive(&m, &chain(30), STREAMING);
        assert!(rec.is_empty(), "no settings from a capture with no speech");
        assert!(rec.needs_remeasure);
    }

    #[test]
    fn clipping_is_called_out() {
        let mut m = measurement(-20.0, -70.0, -0.1);
        m.clipped_fraction = 0.01;

        let rec = derive(&m, &chain(50), STREAMING);
        assert!(rec.notes.iter().any(|n| n.contains("clipped")));
    }

    #[test]
    fn a_wrongly_typed_jack_input_is_flagged() {
        let m = measurement(-20.0, -70.0, -6.0);
        let mut c = chain(50);
        c.mic_type = MicrophoneType::Jack;

        let rec = derive(&m, &c, STREAMING);
        assert!(rec.notes.iter().any(|n| n.contains("Jack")));
    }

    #[test]
    fn more_dynamic_speech_gets_a_higher_ratio() {
        let calm = measurement(-20.0, -70.0, -14.0); // crest 6 dB
        let wild = measurement(-20.0, -70.0, -2.0); // crest 18 dB

        let calm_ratio = derive(&calm, &chain(50), STREAMING).compressor_ratio;
        let wild_ratio = derive(&wild, &chain(50), STREAMING).compressor_ratio;

        assert_ne!(
            calm_ratio, wild_ratio,
            "ratio should follow measured dynamics"
        );
    }

    #[test]
    fn every_target_is_reachable_by_name() {
        for t in crate::targets::ALL {
            assert_eq!(
                crate::targets::by_name(t.name).map(|x| x.name),
                Some(t.name)
            );
        }
        assert!(
            crate::targets::by_name("STREAMING").is_some(),
            "case-insensitive"
        );
        assert!(crate::targets::by_name("nonsense").is_none());
    }
}
