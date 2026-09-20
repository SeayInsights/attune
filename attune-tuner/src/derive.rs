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
use attune_control::settings::{EqBandKey, MicChain};
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

/// Device limits for one equaliser band, in dB.
const EQ_MIN_DB: i32 = -9;
const EQ_MAX_DB: i32 = 9;

/// Largest equaliser move allowed in one pass, per band.
///
/// Same reasoning as the gain step, with an extra one: the measurement is taken
/// *through* the equaliser being adjusted, so each pass changes what the next
/// one sees. Small steps converge; large ones oscillate.
const MAX_EQ_STEP_DB: f32 = 3.0;

/// Ignore band errors smaller than this. Below it the correction is inside the
/// measurement's own noise, and applying it would chase randomness.
const EQ_DEADBAND_DB: f32 = 1.0;

/// One proposed change.
#[derive(Debug, Clone, serde::Serialize)]
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
#[derive(Debug, Clone, serde::Serialize)]
pub struct Recommendation {
    /// Changes to apply, in the order they should be applied.
    pub changes: Vec<Change>,
    /// Concrete values, for the apply path.
    pub gain_db: Option<u16>,
    pub gate_threshold_db: Option<i8>,
    /// How hard the gate ducks when closed, as a percentage.
    pub gate_attenuation: Option<u8>,
    pub compressor_threshold_db: Option<i8>,
    pub compressor_ratio: Option<CompressorRatio>,
    /// Equaliser bands to change, as (band, new gain in dB).
    pub eq: Vec<(EqBandKey, i8)>,
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

/// Format a frequency the way a mixer labels it.
fn format_hz(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{:.0}k", hz / 1000.0)
    } else {
        format!("{hz:.0}")
    }
}

/// The measured level at an arbitrary frequency.
///
/// The measurement's octave bands and the device's equaliser bands do not always
/// share centres -- a Mini's sit at 90 Hz, 3 kHz and so on. Interpolating in log
/// frequency lets one measurement serve any band layout, rather than needing a
/// measurement table per device.
fn measured_level_at(m: &Measurement, hz: f32) -> f32 {
    let bands = &m.bands;
    if bands.is_empty() {
        return 0.0;
    }
    if hz <= bands[0].centre_hz {
        return bands[0].level_db;
    }
    if hz >= bands[bands.len() - 1].centre_hz {
        return bands[bands.len() - 1].level_db;
    }

    for pair in bands.windows(2) {
        let (f0, d0) = (pair[0].centre_hz, pair[0].level_db);
        let (f1, d1) = (pair[1].centre_hz, pair[1].level_db);
        if hz >= f0 && hz <= f1 {
            let t = (hz.ln() - f0.ln()) / (f1.ln() - f0.ln());
            return d0 + t * (d1 - d0);
        }
    }
    0.0
}

/// Derive settings for a target from a measurement of the current chain.
pub fn derive(m: &Measurement, chain: &MicChain, target: Target) -> Recommendation {
    let mut changes = Vec::new();
    let mut notes = Vec::new();

    let mut gain_db = None;
    let mut gate_threshold_db = None;
    let mut gate_attenuation = None;
    let mut compressor_threshold_db = None;
    let mut compressor_ratio = None;
    let mut eq: Vec<(EqBandKey, i8)> = Vec::new();

    let usability = m.usability();
    if usability != attune_analysis::measure::Usability::Usable {
        notes.push(usability.explain());
        return Recommendation {
            changes,
            gain_db,
            gate_threshold_db,
            gate_attenuation,
            compressor_threshold_db,
            compressor_ratio,
            eq,
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

            changes.push(Change {
                setting: "Preamp gain",
                from: format!("{} dB", chain.gain_db),
                to: format!("{proposed} dB"),
                reason,
            });
            gain_db = Some(proposed);
        }

        // Whether the *full* correction is reachable, not just this step.
        // Bounded stepping means the ceiling is not hit in one pass, so
        // checking the proposed value would stay silent until several passes
        // in -- by which point the operator has been told nothing useful.
        //
        // Outside the block above on purpose. When the preamp is already at
        // the maximum, `proposed` equals the current gain, that block does
        // nothing at all, and this is the only thing with anything to say --
        // so having it inside meant the one case with no way forward was also
        // the one case that said nothing. Measured: a 72 dB preamp against a
        // signal 49 dB below target produced no changes, no explanation, and
        // a note telling the operator to apply a gain change that did not
        // exist.
        if chain.gain_db as f32 + level_error > GAIN_MAX_DB as f32 {
            let at_ceiling = chain.gain_db as i32 >= GAIN_MAX_DB;
            let mut note = format!(
                "Even at the device maximum of {GAIN_MAX_DB} dB this signal \
                 stays below target -- it needs about {:.0} dB and the preamp \
                 cannot supply it. The rest has to come from physics: move \
                 closer to the mic, speak up, or check the mic type matches \
                 how the mic is actually connected.",
                chain.gain_db as f32 + level_error
            );

            if at_ceiling {
                note.push_str(&format!(
                    " The preamp is already at {GAIN_MAX_DB} dB, so there is \
                     nothing here to apply: no amount of re-running will \
                     change that until the signal itself comes up."
                ));
            }

            notes.push(note);
        }
    }

    // --- Stage 2 and 3: only once level is right ---------------------------

    if level_off {
        // What comes next depends on whether there is in fact a gain change to
        // apply. Telling someone to "re-measure after applying the gain
        // change" when none was produced sends them round a loop that cannot
        // terminate, and reads as the tool being broken rather than the signal
        // being unfixable from here.
        notes.push(if gain_db.is_some() {
            "Gate and compressor are not being set yet. Both are positions \
             relative to the signal's level, so placing them against an \
             out-of-range signal would put them precisely in the wrong spot. \
             Re-measure after applying the gain change and run again."
                .to_string()
        } else {
            "Gate and compressor are not being set either. Both are positions \
             relative to the signal's level, and placing them against a signal \
             this far out would put them precisely in the wrong spot -- they \
             would have to be moved again the moment the level is fixed. \
             Nothing will be written until it is."
                .to_string()
        });

        return Recommendation {
            changes,
            gain_db,
            gate_threshold_db,
            gate_attenuation,
            compressor_threshold_db,
            compressor_ratio,
            eq,
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

        // A gate that ducks is not a gate. 100% is the only setting that
        // actually silences the signal between words; anything less leaves the
        // room audible, and at the preamp gains a dynamic mic needs, clearly
        // so. Found on real hardware set to 85%, where it reads as the gate
        // simply not working.
        if chain.gate_attenuation < 100 {
            changes.push(Change {
                setting: "Gate attenuation",
                from: format!("{}%", chain.gate_attenuation),
                to: "100%".to_string(),
                reason: format!(
                    "The gate is set to duck by {}% rather than mute. Between \
                     words the room still comes through at {}% of its level, \
                     which with a preamp this high is plainly audible -- and \
                     reads as the gate doing nothing rather than as a setting.",
                    chain.gate_attenuation,
                    100 - chain.gate_attenuation
                ),
            });
            gate_attenuation = Some(100);
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

    // --- Stage 4: equaliser -------------------------------------------------
    //
    // The measurement is taken *through* the equaliser, so a band's correction
    // is relative: new = current + (target - measured). Treating the measured
    // curve as the raw microphone would double-apply whatever is already set.

    if !m.bands.is_empty() && !chain.eq.is_empty() {
        let mut moves: Vec<String> = Vec::new();

        for band in &chain.eq {
            let measured = measured_level_at(m, band.centre_hz);
            let wanted = target.level_at(band.centre_hz);
            let error = wanted - measured;

            if error.abs() < EQ_DEADBAND_DB {
                continue;
            }

            let step = error.clamp(-MAX_EQ_STEP_DB, MAX_EQ_STEP_DB);
            let proposed =
                ((band.gain_db as f32 + step).round() as i32).clamp(EQ_MIN_DB, EQ_MAX_DB) as i8;

            if proposed == band.gain_db {
                continue;
            }

            eq.push((band.key, proposed));
            moves.push(format!(
                "{} {:+} dB",
                format_hz(band.centre_hz),
                proposed - band.gain_db
            ));
        }

        if !eq.is_empty() {
            changes.push(Change {
                setting: "Microphone EQ",
                from: format!("{} band(s)", eq.len()),
                to: moves.join(", "),
                reason: format!(
                    "Measured against the '{}' voice curve. Corrections are \
                     relative to what the equaliser is already doing, because \
                     the measurement passes through it -- and each band moves \
                     at most {:.0} dB per pass, since changing the equaliser \
                     changes what the next measurement sees.",
                    target.name, MAX_EQ_STEP_DB
                ),
            });
        }
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
        gate_attenuation,
        compressor_threshold_db,
        compressor_ratio,
        eq,
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
            bands: Vec::new(),
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
            eq: Vec::new(),
        }
    }

    /// Regression. A preamp already at the device maximum, against a signal
    /// still far below target, produced: no changes, no gain, no explanation,
    /// and a note telling the operator to apply a gain change that was never
    /// made. Pressing "Measure and apply" therefore did nothing, for ever,
    /// with nothing on screen saying why.
    ///
    /// Found on real hardware -- a 72 dB preamp and speech at -68.9 dBFS.
    #[test]
    fn a_preamp_pinned_at_maximum_says_so_instead_of_going_quiet() {
        let m = measurement(-68.9, -72.9, -58.5);
        let r = derive(&m, &chain(GAIN_MAX_DB as u16), STREAMING);

        assert!(r.gain_db.is_none(), "there is no gain left to give");
        assert!(r.is_empty(), "nothing can be applied");

        let notes = r.notes.join(" ");
        assert!(
            notes.contains(&GAIN_MAX_DB.to_string()),
            "the ceiling is never mentioned: {notes}"
        );
        assert!(
            notes.contains("nothing here to apply"),
            "it does not say that pressing apply will do nothing: {notes}"
        );
        // And it must not send someone round a loop that cannot terminate.
        assert!(
            !notes.contains("after applying the gain change"),
            "it still tells the operator to apply a change that does not exist: {notes}"
        );
    }

    /// The same signal with headroom left in the preamp must still propose a
    /// step -- the fix above must not have turned every quiet capture into a
    /// dead end.
    #[test]
    fn a_preamp_below_maximum_still_gets_a_step() {
        let m = measurement(-68.9, -72.9, -58.5);
        let r = derive(&m, &chain(40), STREAMING);

        assert_eq!(r.gain_db, Some(52), "expected a bounded 12 dB step");
        assert!(!r.is_empty());

        // It should still warn that the ceiling will not be enough, because it
        // will not be: 40 + 48.9 is past 72.
        let notes = r.notes.join(" ");
        assert!(notes.contains("device maximum"), "{notes}");
        assert!(
            notes.contains("after applying the gain change"),
            "there IS a change to apply here, so it should say so: {notes}"
        );
    }

    /// A gate that ducks is not a gate. Found on real hardware at 85%, where
    /// the room stayed audible between words and read as the gate simply not
    /// working -- the one symptom the tuner had no way to address, because
    /// attenuation was not something it could set at all.
    #[test]
    fn a_ducking_gate_is_brought_up_to_a_real_one() {
        let mut c = chain(40);
        c.gate_attenuation = 85;
        // On-target level, so the gate stage is actually reached.
        let m = measurement(-20.0, -60.0, -9.0);

        let r = derive(&m, &c, STREAMING);
        assert_eq!(r.gate_attenuation, Some(100));
        assert!(
            r.changes.iter().any(|ch| ch.setting == "Gate attenuation"),
            "the change is not shown to the operator"
        );
    }

    /// And a gate already muting is left alone rather than rewritten every run.
    #[test]
    fn a_gate_already_muting_is_not_touched() {
        let mut c = chain(40);
        c.gate_attenuation = 100;
        let m = measurement(-20.0, -60.0, -9.0);

        let r = derive(&m, &c, STREAMING);
        assert_eq!(r.gate_attenuation, None);
    }

    /// A chain with a flat ten-band equaliser, for the EQ tests.
    fn chain_with_flat_eq(gain: u16) -> MicChain {
        use attune_control::settings::EqBand;
        use goxlr_types::EqFrequencies::*;

        let bands = [
            (31.5, Equalizer31Hz),
            (63.0, Equalizer63Hz),
            (125.0, Equalizer125Hz),
            (250.0, Equalizer250Hz),
            (500.0, Equalizer500Hz),
            (1000.0, Equalizer1KHz),
            (2000.0, Equalizer2KHz),
            (4000.0, Equalizer4KHz),
            (8000.0, Equalizer8KHz),
            (16000.0, Equalizer16KHz),
        ];

        let mut c = chain(gain);
        c.eq = bands
            .into_iter()
            .map(|(hz, key)| EqBand {
                centre_hz: hz,
                gain_db: 0,
                key: EqBandKey::Full(key),
            })
            .collect();
        c
    }

    /// A measurement whose spectrum is flat across every band.
    fn flat_spectrum(mut m: Measurement) -> Measurement {
        use attune_analysis::measure::{BAND_CENTRES_HZ, Band};
        m.bands = BAND_CENTRES_HZ
            .iter()
            .map(|hz| Band {
                centre_hz: *hz,
                level_db: 0.0,
            })
            .collect();
        m
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

    // --- Equaliser ---------------------------------------------------------

    /// A flat mic against a curve that wants low cut and presence lift should
    /// get exactly that shape, not a uniform move.
    #[test]
    fn a_flat_spectrum_is_shaped_toward_the_target_curve() {
        let m = flat_spectrum(measurement(-20.0, -70.0, -6.0));
        let rec = derive(&m, &chain_with_flat_eq(50), STREAMING);

        assert!(!rec.eq.is_empty(), "a flat mic should get EQ corrections");

        let by_key: std::collections::HashMap<_, _> = rec.eq.iter().copied().collect();
        let low = by_key[&EqBandKey::Full(goxlr_types::EqFrequencies::Equalizer31Hz)];
        let presence = by_key[&EqBandKey::Full(goxlr_types::EqFrequencies::Equalizer2KHz)];

        assert!(low < 0, "31 Hz should be cut, got {low:+}");
        assert!(presence > 0, "2 kHz should be lifted, got {presence:+}");
    }

    /// The safety property, same as gain: one pass cannot make a large move.
    #[test]
    fn eq_moves_by_at_most_one_bounded_step_per_band() {
        let m = flat_spectrum(measurement(-20.0, -70.0, -6.0));
        let rec = derive(&m, &chain_with_flat_eq(50), STREAMING);

        for (key, gain) in &rec.eq {
            assert!(
                (*gain as f32).abs() <= MAX_EQ_STEP_DB,
                "{key:?} moved to {gain:+} dB from 0 in one pass, cap is {MAX_EQ_STEP_DB}"
            );
        }
    }

    /// EQ is a relative correction, so a chain already matching the curve
    /// should be left alone rather than re-corrected every pass.
    #[test]
    fn a_spectrum_already_on_the_curve_gets_no_eq_change() {
        let mut m = measurement(-20.0, -70.0, -6.0);
        use attune_analysis::measure::{BAND_CENTRES_HZ, Band};
        m.bands = BAND_CENTRES_HZ
            .iter()
            .map(|hz| Band {
                centre_hz: *hz,
                level_db: STREAMING.level_at(*hz),
            })
            .collect();

        let rec = derive(&m, &chain_with_flat_eq(50), STREAMING);
        assert!(
            rec.eq.is_empty(),
            "already on curve, but proposed {:?}",
            rec.eq
        );
    }

    #[test]
    fn no_eq_is_proposed_while_level_is_still_wrong() {
        let m = flat_spectrum(measurement(-46.0, -71.0, -32.0));
        let rec = derive(&m, &chain_with_flat_eq(30), STREAMING);
        assert!(rec.eq.is_empty(), "level comes first");
    }

    #[test]
    fn a_device_reporting_no_equaliser_is_handled() {
        let m = flat_spectrum(measurement(-20.0, -70.0, -6.0));
        let rec = derive(&m, &chain(50), STREAMING);
        assert!(rec.eq.is_empty());
    }

    /// The target curve must answer for the Mini's centres too, which do not
    /// match the measurement's octave bands.
    #[test]
    fn the_voice_curve_interpolates_to_arbitrary_frequencies() {
        for hz in [90.0, 3000.0, 6300.0] {
            let v = STREAMING.level_at(hz);
            assert!(v.is_finite(), "no value at {hz} Hz");
            assert!((-20.0..=10.0).contains(&v), "{hz} Hz gave {v} dB");
        }
        // Interpolation stays between its anchors.
        let at_3k = STREAMING.level_at(3000.0);
        assert!(
            at_3k <= STREAMING.level_at(2000.0).max(STREAMING.level_at(4000.0)),
            "3 kHz should sit between its neighbours"
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
