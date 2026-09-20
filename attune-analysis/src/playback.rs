//! Playing a test signal to a named output device.
//!
//! Exists to answer one question that cannot be answered by reading
//! configuration: *is the correction actually being applied?*
//!
//! Equalizer APO matches its `Device:` directive against names Windows reports,
//! and a directive that does not match fails silently -- no error, no log, just
//! no effect. Playing a known signal through a bus and measuring what comes back
//! turns that from a guess into a measurement.
//!
//! The GoXLR makes this possible: its Stream Mix bus is a capture device
//! carrying the mixed output, so audio sent to Game can be recorded again after
//! the whole chain has run.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::capture::CaptureError;

/// Generate pink-ish noise.
///
/// Pink rather than white because it has equal energy per octave, so every
/// octave band of the measurement gets a comparable amount of signal. White
/// noise leaves the low bands starved and their measured levels noisy.
///
/// The Voss-McCartney approximation: cheap, and accurate enough for comparing a
/// band against itself before and after a correction.
pub fn pink_noise(samples: usize, seed: u32) -> Vec<f32> {
    const ROWS: usize = 8;
    let mut rows = [0.0f32; ROWS];
    let mut running = 0.0f32;
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(samples);

    let next = |state: &mut u32| -> f32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((*state >> 8) as f32 / 8_388_608.0) - 1.0
    };

    for i in 0..samples {
        // Each row updates at half the rate of the one before it.
        let mut n = i;
        for (row, slot) in rows.iter_mut().enumerate() {
            if row == ROWS - 1 || n & 1 == 1 {
                running -= *slot;
                *slot = next(&mut state);
                running += *slot;
                break;
            }
            n >>= 1;
        }
        // Scale well below full scale: the point is to measure a response, and
        // a hot test signal risks clipping whatever it passes through.
        out.push((running / ROWS as f32 + next(&mut state) / ROWS as f32) * 0.25);
    }

    out
}

fn find_output(needle: &str) -> Result<cpal::Device, CaptureError> {
    let host = cpal::default_host();
    let needle_lower = needle.to_lowercase();

    let devices = host
        .output_devices()
        .map_err(|e| CaptureError::DeviceUnavailable {
            device: needle.to_string(),
            source: anyhow::anyhow!(e),
        })?;

    for device in devices {
        if let Ok(name) = device.name()
            && name.to_lowercase().contains(&needle_lower)
        {
            return Ok(device);
        }
    }

    Err(CaptureError::DeviceNotFound(
        needle.to_string(),
        crate::capture::list_output_devices().join(", "),
    ))
}

/// Play `samples` to the first output device matching `needle`, and return when
/// the buffer has been consumed.
///
/// Mono input is fanned out to every channel the device wants.
pub fn play(needle: &str, samples: &[f32], sample_rate_hint: u32) -> Result<(), CaptureError> {
    let device = find_output(needle)?;
    let device_name = device.name().unwrap_or_else(|_| needle.to_string());

    let config = device
        .default_output_config()
        .map_err(|e| CaptureError::DeviceUnavailable {
            device: device_name.clone(),
            source: anyhow::anyhow!(e),
        })?;

    let channels = config.channels() as usize;
    let device_rate = config.sample_rate().0;

    // Nearest-neighbour resample. Adequate for noise, whose exact sample timing
    // carries no information; anything tonal would need better.
    let ratio = sample_rate_hint as f64 / device_rate as f64;
    let total_out = ((samples.len() as f64) / ratio) as usize;

    let cursor = Arc::new(Mutex::new(0usize));
    let source: Arc<Vec<f32>> = Arc::new(samples.to_vec());

    let cursor_cb = Arc::clone(&cursor);
    let source_cb = Arc::clone(&source);

    let stream = device
        .build_output_stream(
            &config.config(),
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let Ok(mut pos) = cursor_cb.lock() else {
                    out.fill(0.0);
                    return;
                };
                for frame in out.chunks_mut(channels) {
                    let index = ((*pos as f64) * ratio) as usize;
                    let value = source_cb.get(index).copied().unwrap_or(0.0);
                    for sample in frame.iter_mut() {
                        *sample = value;
                    }
                    *pos += 1;
                }
            },
            move |_err| {},
            None,
        )
        .map_err(|e| CaptureError::DeviceUnavailable {
            device: device_name.clone(),
            source: anyhow::anyhow!(e),
        })?;

    stream.play().map_err(|e| CaptureError::DeviceUnavailable {
        device: device_name.clone(),
        source: anyhow::anyhow!(e),
    })?;

    // Wait on wall clock rather than on the cursor: if the device stalls, this
    // returns rather than hanging a request forever.
    let expected = Duration::from_secs_f64(total_out as f64 / device_rate as f64);
    let deadline = Instant::now() + expected + Duration::from_millis(500);
    while Instant::now() < deadline {
        if cursor.lock().map(|p| *p >= total_out).unwrap_or(true) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    drop(stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::measure;

    #[test]
    fn pink_noise_is_not_silent_and_stays_in_range() {
        let n = pink_noise(48_000, 1);
        assert_eq!(n.len(), 48_000);
        assert!(n.iter().any(|s| s.abs() > 0.01), "signal is silent");
        assert!(
            n.iter().all(|s| s.abs() <= 1.0),
            "signal exceeds full scale"
        );
    }

    /// Pink noise has constant energy per octave, so its energy **per Hz** falls
    /// as frequency rises. The measurement reports density, so pink reads as a
    /// descending slope, not as flat.
    ///
    /// An earlier version of this test asserted flatness, with a comment
    /// claiming the two facts cancelled. They do not -- that was simply
    /// confused, and the threshold was loose enough to hide it.
    ///
    /// None of this affects the verification the generator exists for: that
    /// compares a band against itself with a correction on and off, and the
    /// source spectrum cancels out of the difference. The signal only has to
    /// put energy in every band, which is what pink does better than white.
    #[test]
    fn pink_noise_has_energy_in_every_band_and_descends() {
        let m = measure(&pink_noise(48_000 * 2, 7), 48_000);

        let bands: Vec<(f32, f32)> = m
            .bands
            .iter()
            .filter(|b| b.centre_hz >= 125.0 && b.centre_hz <= 8000.0)
            .map(|b| (b.centre_hz, b.level_db))
            .collect();
        assert!(bands.len() >= 6, "expected most bands present");

        // Every band carries real signal rather than numerical floor.
        for (hz, db) in &bands {
            assert!(
                *db > -40.0,
                "{hz:.0} Hz was effectively empty at {db:.1} dB"
            );
        }

        let first = bands.first().unwrap().1;
        let last = bands.last().unwrap().1;
        assert!(
            first > last,
            "density should fall with frequency: {first:.1} dB at the bottom, \
             {last:.1} dB at the top"
        );
    }
}
