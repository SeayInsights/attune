//! Turning a capture into numbers.
//!
//! All levels are dBFS: 0 dB is full scale, everything real is negative.
//!
//! Percentiles rather than means throughout. A mean over a speech recording is
//! dominated by the silence between words, which is why naive level metering
//! reports a number nobody recognises. The 10th percentile of frame energy is a
//! good stand-in for the noise floor, and the 90th for speech level.

use rustfft::{FftPlanner, num_complex::Complex};

/// Analysis frame length. 20 ms is long enough for a stable low-frequency
/// estimate and short enough to separate speech from the gaps between words.
const FRAME_MS: f64 = 20.0;

/// Largest margin above the noise floor used to separate speech from silence.
///
/// The actual margin adapts: it is half the measured signal-to-noise ratio,
/// capped here. A fixed margin looks reasonable and is wrong -- on a steady
/// signal the 10th and 90th percentiles coincide, so a fixed offset above the
/// floor excludes every frame and the spectrum comes back empty. Scaling with
/// the range means a signal with no dynamics admits all of its frames, and real
/// speech still rejects the gaps between words.
const SPEECH_MARGIN_MAX_DB: f32 = 12.0;

/// Anything at or above this counts as clipped.
const CLIP_THRESHOLD: f32 = 0.999;

/// No frame below this can be speech, whatever the relative maths says.
///
/// Without this, digital silence qualifies: every frame sits at the floor, the
/// adaptive margin collapses to zero, and a recording of nothing reports a
/// confident spectrum. An absolute limit is the only thing that distinguishes
/// "quiet" from "not there".
const ABSOLUTE_SILENCE_DBFS: f32 = -80.0;

/// Octave band centres, matching the GoXLR's equaliser bands.
///
/// The device's own labels are 31.5 Hz through 16 kHz. Measuring on the same
/// centres means a correction maps onto a band one-to-one, with no interpolation
/// inventing values between them.
pub const BAND_CENTRES_HZ: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// One octave band of the measured spectrum.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Band {
    /// Band centre in Hz.
    pub centre_hz: f32,
    /// Level relative to this capture's average spectral density, in dB.
    ///
    /// **Density, not total energy.** An octave band at 8 kHz is thirty times
    /// wider than one at 250 Hz, so comparing raw sums across them says more
    /// about bandwidth than about tone. Dividing by each band's width in Hz is
    /// what makes these numbers comparable to each other, and it is the whole
    /// reason this replaced an earlier three-band version that did not.
    pub level_db: f32,
}

/// What a capture tells us about the signal.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Measurement {
    /// Capture length in seconds.
    pub duration_secs: f64,
    /// Sample rate of the capture.
    pub sample_rate: u32,

    /// 10th percentile of frame RMS. The room and the preamp, with no one talking.
    pub noise_floor_dbfs: f32,
    /// 90th percentile of frame RMS. Representative speech level.
    pub speech_level_dbfs: f32,
    /// Highest single sample in the capture.
    pub peak_dbfs: f32,

    /// Speech level minus noise floor. How much room the gate has to work in.
    pub signal_to_noise_db: f32,
    /// Peak minus speech level. High means uncontrolled dynamics.
    pub crest_factor_db: f32,

    /// Fraction of samples at or beyond full scale.
    pub clipped_fraction: f32,

    /// The spectrum as octave bands, centred on the frequencies the GoXLR's
    /// equaliser exposes. Empty when there was not enough speech to measure.
    pub bands: Vec<Band>,

    /// How many frames were classified as speech. Low means a quiet recording,
    /// and every spectral figure above should be distrusted accordingly.
    pub speech_frames: usize,
    /// Total frames analysed.
    pub total_frames: usize,
}

/// Minimum absolute speech frames.
const MIN_SPEECH_FRAMES: usize = 20;

/// Minimum proportion of the capture that must contain speech.
///
/// An absolute count alone is not enough, and getting this wrong has already
/// caused real damage: a capture with 40 speech frames out of 750 cleared a
/// count-only check, and the tuner then derived a 42 dB gain correction from
/// what was effectively silence and pinned a preamp to its maximum. Five percent
/// of a recording is not a measurement of a voice.
const MIN_SPEECH_FRACTION: f32 = 0.15;

/// No working microphone carrying a human voice measures below this.
///
/// Anything quieter is a room being recorded, not a person, however many frames
/// happened to clear the relative threshold.
const MIN_PLAUSIBLE_SPEECH_DBFS: f32 = -70.0;

/// Why a measurement can or cannot be used.
#[derive(Debug, Clone, PartialEq)]
pub enum Usability {
    /// Enough speech, at a plausible level.
    Usable,
    /// Too little of the capture contained speech.
    TooLittleSpeech { frames: usize, total: usize },
    /// Speech was detected but at a level no real voice produces.
    ImplausiblyQuiet { level_dbfs: f32 },
}

impl Usability {
    /// One line explaining the verdict, for the operator.
    pub fn explain(&self) -> String {
        match self {
            Usability::Usable => "Capture is usable.".to_string(),
            Usability::TooLittleSpeech { frames, total } => format!(
                "Only {frames} of {total} frames contained speech ({:.0}%). \
                 Nothing can be derived from this. Record again and speak \
                 throughout -- the recording does not wait for you to start.",
                (*frames as f32 / *total.max(&1) as f32) * 100.0
            ),
            Usability::ImplausiblyQuiet { level_dbfs } => format!(
                "Speech measured {level_dbfs:.1} dBFS, which is below anything a \
                 working microphone produces for a human voice. This is a \
                 recording of a room. Check the mic is connected, unmuted, and \
                 that the right input device was captured."
            ),
        }
    }
}

impl Measurement {
    /// Whether this measurement can be reasoned from.
    pub fn usability(&self) -> Usability {
        let fraction = self.speech_frames as f32 / self.total_frames.max(1) as f32;

        if self.speech_frames < MIN_SPEECH_FRAMES || fraction < MIN_SPEECH_FRACTION {
            return Usability::TooLittleSpeech {
                frames: self.speech_frames,
                total: self.total_frames,
            };
        }

        if self.speech_level_dbfs < MIN_PLAUSIBLE_SPEECH_DBFS {
            return Usability::ImplausiblyQuiet {
                level_dbfs: self.speech_level_dbfs,
            };
        }

        Usability::Usable
    }

    /// Whether there was enough speech for the figures to mean anything.
    pub fn has_usable_speech(&self) -> bool {
        self.usability() == Usability::Usable
    }
}

fn to_db(linear: f32) -> f32 {
    if linear <= 1e-10 {
        -100.0
    } else {
        20.0 * linear.log10()
    }
}

fn percentile(sorted: &[f32], p: f64) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index]
}

/// Measure a captured buffer.
pub fn measure(samples: &[f32], sample_rate: u32) -> Measurement {
    let frame_len = ((sample_rate as f64) * FRAME_MS / 1000.0) as usize;
    let frame_len = frame_len.max(64);

    // --- Per-frame RMS ----------------------------------------------------

    let mut frame_rms: Vec<f32> = Vec::new();
    for frame in samples.chunks(frame_len) {
        if frame.len() < frame_len {
            break;
        }
        let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
        frame_rms.push((sum_sq / frame.len() as f32).sqrt());
    }

    let mut sorted = frame_rms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let noise_floor = percentile(&sorted, 0.10);
    let speech_level = percentile(&sorted, 0.90);
    let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));

    let noise_floor_dbfs = to_db(noise_floor);
    let speech_level_dbfs = to_db(speech_level);
    let peak_dbfs = to_db(peak);

    let clipped = samples.iter().filter(|s| s.abs() >= CLIP_THRESHOLD).count();
    let clipped_fraction = clipped as f32 / samples.len().max(1) as f32;

    // --- Spectrum over speech frames only ---------------------------------

    let snr = speech_level_dbfs - noise_floor_dbfs;
    let speech_gate = (noise_floor_dbfs + (0.5 * snr).clamp(0.0, SPEECH_MARGIN_MAX_DB))
        .max(ABSOLUTE_SILENCE_DBFS);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(frame_len);

    let mut spectrum_sum = vec![0.0f64; frame_len / 2];
    let mut speech_frames = 0usize;

    for (index, frame) in samples.chunks(frame_len).enumerate() {
        if frame.len() < frame_len || index >= frame_rms.len() {
            break;
        }
        if to_db(frame_rms[index]) < speech_gate {
            continue;
        }
        speech_frames += 1;

        // Hann window, so band energies are not smeared by frame edges.
        let mut buffer: Vec<Complex<f32>> = frame
            .iter()
            .enumerate()
            .map(|(n, s)| {
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / frame_len as f32).cos();
                Complex::new(s * w, 0.0)
            })
            .collect();

        fft.process(&mut buffer);

        for (bin, value) in buffer.iter().take(frame_len / 2).enumerate() {
            spectrum_sum[bin] += value.norm() as f64;
        }
    }

    let bands = octave_bands(&spectrum_sum, sample_rate, frame_len);

    Measurement {
        duration_secs: samples.len() as f64 / sample_rate as f64,
        sample_rate,
        noise_floor_dbfs,
        speech_level_dbfs,
        peak_dbfs,
        signal_to_noise_db: speech_level_dbfs - noise_floor_dbfs,
        crest_factor_db: peak_dbfs - speech_level_dbfs,
        clipped_fraction,
        bands,
        speech_frames,
        total_frames: frame_rms.len(),
    }
}

/// Reduce a linear magnitude spectrum to octave bands of comparable density.
///
/// Returns levels relative to the mean band density, so the result describes the
/// signal's *tilt* rather than its loudness. That is what a corrective EQ needs:
/// how the energy is distributed, independent of how much of it there is.
fn octave_bands(spectrum: &[f64], sample_rate: u32, frame_len: usize) -> Vec<Band> {
    if spectrum.is_empty() {
        return Vec::new();
    }

    let bin_hz = sample_rate as f64 / frame_len as f64;
    let nyquist = sample_rate as f64 / 2.0;

    // Octave band edges: centre / sqrt(2) to centre * sqrt(2).
    const HALF_OCTAVE: f64 = std::f64::consts::SQRT_2;

    let mut densities: Vec<(f32, f64)> = Vec::with_capacity(BAND_CENTRES_HZ.len());

    for centre in BAND_CENTRES_HZ {
        let centre = centre as f64;
        let low = centre / HALF_OCTAVE;
        let high = (centre * HALF_OCTAVE).min(nyquist);

        if low >= high {
            // Band sits above Nyquist for this sample rate. Omitting it is
            // honest; reporting a level for a band that was not sampled is not.
            continue;
        }

        let low_bin = (low / bin_hz).floor() as usize;
        let high_bin = ((high / bin_hz).ceil() as usize).min(spectrum.len());
        if low_bin >= high_bin {
            continue;
        }

        let energy: f64 = spectrum[low_bin..high_bin].iter().sum();
        let density = energy / (high - low);
        densities.push((centre as f32, density));
    }

    if densities.is_empty() {
        return Vec::new();
    }

    // Reference is the mean density across bands, so the curve is centred and a
    // flat signal reads as flat regardless of overall level.
    let mean: f64 = densities.iter().map(|(_, d)| *d).sum::<f64>() / densities.len() as f64;

    densities
        .into_iter()
        .map(|(centre_hz, density)| Band {
            centre_hz,
            level_db: if mean <= 0.0 || density <= 0.0 {
                -60.0
            } else {
                (10.0 * (density / mean).log10()) as f32
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, secs: f32, rate: u32, amplitude: f32) -> Vec<f32> {
        let n = (secs * rate as f32) as usize;
        (0..n)
            .map(|i| amplitude * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn full_scale_sine_measures_near_zero_dbfs_peak() {
        let m = measure(&sine(1000.0, 1.0, 48000, 1.0), 48000);
        assert!(m.peak_dbfs > -0.5, "peak was {}", m.peak_dbfs);
    }

    #[test]
    fn half_amplitude_sine_measures_near_minus_six_dbfs() {
        let m = measure(&sine(1000.0, 1.0, 48000, 0.5), 48000);
        assert!(
            (m.peak_dbfs - -6.02).abs() < 0.5,
            "peak was {}",
            m.peak_dbfs
        );
    }

    /// The loudest band should be the one the tone actually sits in.
    fn loudest_band(m: &Measurement) -> f32 {
        m.bands
            .iter()
            .max_by(|a, b| {
                a.level_db
                    .partial_cmp(&b.level_db)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|b| b.centre_hz)
            .unwrap_or(0.0)
    }

    #[test]
    fn a_tone_lands_in_its_own_octave_band() {
        for centre in [125.0, 500.0, 2000.0, 8000.0] {
            let m = measure(&sine(centre, 1.0, 48000, 0.5), 48000);
            assert_eq!(
                loudest_band(&m),
                centre,
                "a {centre} Hz tone should peak in the {centre} Hz band"
            );
        }
    }

    /// The reason the three-band version was replaced: an octave at 8 kHz is
    /// thirty times wider than one at 250 Hz, so equal *density* must read as
    /// equal level rather than the wide band winning on width alone.
    #[test]
    fn band_levels_are_density_so_width_does_not_decide_them() {
        // White-ish noise has roughly flat density across the spectrum.
        let mut seed = 12345u32;
        let noise: Vec<f32> = (0..48000 * 2)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 8) as f32 / 8388608.0) - 1.0
            })
            .collect();

        let m = measure(&noise, 48000);
        let mids: Vec<f32> = m
            .bands
            .iter()
            .filter(|b| b.centre_hz >= 250.0 && b.centre_hz <= 8000.0)
            .map(|b| b.level_db)
            .collect();

        let spread = mids.iter().cloned().fold(f32::MIN, f32::max)
            - mids.iter().cloned().fold(f32::MAX, f32::min);

        assert!(
            spread < 6.0,
            "flat noise should read flat across bands, spread was {spread:.1} dB"
        );
    }

    #[test]
    fn bands_above_nyquist_are_omitted_not_invented() {
        // At 16 kHz sample rate, Nyquist is 8 kHz: the 16 kHz band cannot exist.
        let m = measure(&sine(1000.0, 1.0, 16000, 0.5), 16000);
        assert!(
            !m.bands.iter().any(|b| b.centre_hz == 16000.0),
            "a band above Nyquist must not be reported"
        );
    }

    #[test]
    fn silence_reports_no_usable_speech_rather_than_plausible_numbers() {
        let m = measure(&vec![0.0; 48000], 48000);
        assert!(!m.has_usable_speech());
    }

    #[test]
    fn clipping_is_detected() {
        let mut samples = sine(1000.0, 1.0, 48000, 1.0);
        for s in samples.iter_mut() {
            *s = s.clamp(-1.0, 1.0) * 1.5;
            *s = s.clamp(-1.0, 1.0);
        }
        let m = measure(&samples, 48000);
        assert!(m.clipped_fraction > 0.0);
    }
}
