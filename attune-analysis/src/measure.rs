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

/// What a capture tells us about the signal.
#[derive(Debug, Clone)]
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

    /// Energy 200-400 Hz relative to total speech energy, in dB. Boxiness.
    pub low_mid_db: f32,
    /// Energy 2-5 kHz relative to total, in dB. Intelligibility and presence.
    pub presence_db: f32,
    /// Energy 5-9 kHz relative to total, in dB. Sibilance.
    pub sibilance_db: f32,

    /// How many frames were classified as speech. Low means a quiet recording,
    /// and every spectral figure above should be distrusted accordingly.
    pub speech_frames: usize,
    /// Total frames analysed.
    pub total_frames: usize,
}

impl Measurement {
    /// Whether there was enough speech for the spectral figures to mean anything.
    ///
    /// Reported rather than silently assumed, because a measurement taken in
    /// silence produces numbers that look perfectly plausible.
    pub fn has_usable_speech(&self) -> bool {
        self.speech_frames >= 20
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

    let bin_hz = sample_rate as f64 / frame_len as f64;
    let band_energy = |low_hz: f64, high_hz: f64| -> f64 {
        let low_bin = (low_hz / bin_hz).floor() as usize;
        let high_bin = ((high_hz / bin_hz).ceil() as usize).min(spectrum_sum.len());
        if low_bin >= high_bin {
            return 0.0;
        }
        spectrum_sum[low_bin..high_bin].iter().sum()
    };

    let total: f64 = spectrum_sum.iter().sum();
    let relative_db = |low: f64, high: f64| -> f32 {
        if total <= 0.0 {
            return -100.0;
        }
        let ratio = band_energy(low, high) / total;
        if ratio <= 1e-10 {
            -100.0
        } else {
            (20.0 * ratio.log10()) as f32
        }
    };

    Measurement {
        duration_secs: samples.len() as f64 / sample_rate as f64,
        sample_rate,
        noise_floor_dbfs,
        speech_level_dbfs,
        peak_dbfs,
        signal_to_noise_db: speech_level_dbfs - noise_floor_dbfs,
        crest_factor_db: peak_dbfs - speech_level_dbfs,
        clipped_fraction,
        low_mid_db: relative_db(200.0, 400.0),
        presence_db: relative_db(2000.0, 5000.0),
        sibilance_db: relative_db(5000.0, 9000.0),
        speech_frames,
        total_frames: frame_rms.len(),
    }
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

    #[test]
    fn a_three_khz_tone_lands_in_the_presence_band_not_sibilance() {
        let m = measure(&sine(3000.0, 1.0, 48000, 0.5), 48000);
        assert!(
            m.presence_db > m.sibilance_db,
            "presence {} should exceed sibilance {}",
            m.presence_db,
            m.sibilance_db
        );
    }

    #[test]
    fn a_seven_khz_tone_lands_in_the_sibilance_band() {
        let m = measure(&sine(7000.0, 1.0, 48000, 0.5), 48000);
        assert!(
            m.sibilance_db > m.presence_db,
            "sibilance {} should exceed presence {}",
            m.sibilance_db,
            m.presence_db
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
