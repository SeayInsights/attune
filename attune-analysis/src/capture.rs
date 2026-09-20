//! Fixed-duration capture from a named input device.
//!
//! Deliberately simple: open the device, collect samples for a set duration,
//! stop, hand back a buffer. No streaming, no callbacks that outlive the call.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Things that can go wrong before any audio is collected.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// No input device matched.
    #[error("no input device matching '{0}'. Available: {1}")]
    DeviceNotFound(String, String),

    /// The device exists but would not open with a usable configuration.
    #[error("could not open '{device}': {source}")]
    DeviceUnavailable {
        device: String,
        #[source]
        source: anyhow::Error,
    },

    /// The capture ran but produced nothing.
    #[error("captured no audio from '{0}' -- is the device muted or in use exclusively?")]
    Silent(String),

    /// The audio backend reported an error while the stream was running.
    #[error("audio stream error on '{device}': {message}")]
    StreamFailed { device: String, message: String },
}

/// A finished capture.
pub struct Capture {
    /// Mono samples, normalised to -1.0..=1.0.
    pub samples: Vec<f32>,
    /// Sample rate the device actually gave us, which may not be what was asked for.
    pub sample_rate: u32,
    /// Name of the device it came from.
    pub device_name: String,
}

impl Capture {
    /// Duration of the capture.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}

/// Every input device the host can see.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devices) => devices.filter_map(|d| d.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Find an input device whose name contains `needle`, case-insensitively.
///
/// Substring rather than exact match because Windows device names carry the
/// driver's own decoration, which changes between driver versions.
fn find_input(needle: &str) -> Result<cpal::Device, CaptureError> {
    let host = cpal::default_host();
    let needle_lower = needle.to_lowercase();

    let devices = host
        .input_devices()
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
        list_input_devices().join(", "),
    ))
}

/// Record from the first input device matching `needle` for `duration`.
///
/// Multi-channel input is downmixed to mono by averaging, because everything
/// downstream is a single-channel measurement of one microphone.
pub fn record(needle: &str, duration: Duration) -> Result<Capture, CaptureError> {
    let device = find_input(needle)?;
    let device_name = device.name().unwrap_or_else(|_| needle.to_string());

    let config = device
        .default_input_config()
        .map_err(|e| CaptureError::DeviceUnavailable {
            device: device_name.clone(),
            source: anyhow::anyhow!(e),
        })?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;

    let collected: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(
        (sample_rate as usize) * duration.as_secs() as usize + sample_rate as usize,
    )));

    let sink = Arc::clone(&collected);
    let err_sink = Arc::new(Mutex::new(None::<String>));
    let err_for_cb = Arc::clone(&err_sink);

    let stream = device
        .build_input_stream(
            &config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Downmix to mono. Lock contention here is irrelevant -- the
                // consumer does not touch the buffer until the stream is dropped.
                if let Ok(mut buffer) = sink.lock() {
                    for frame in data.chunks(channels) {
                        let sum: f32 = frame.iter().sum();
                        buffer.push(sum / channels as f32);
                    }
                }
            },
            move |err| {
                if let Ok(mut slot) = err_for_cb.lock() {
                    *slot = Some(err.to_string());
                }
            },
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

    // Busy-wait on wall clock rather than sleeping the exact duration: the
    // stream fills asynchronously and we want to stop on elapsed time, not on
    // an assumed sample count.
    let started = Instant::now();
    while started.elapsed() < duration {
        std::thread::sleep(Duration::from_millis(20));
    }

    drop(stream);

    // A mid-stream backend error means the samples we did collect are suspect.
    // Surfacing it beats handing back a short buffer that looks fine.
    if let Ok(slot) = err_sink.lock()
        && let Some(message) = slot.clone()
    {
        return Err(CaptureError::StreamFailed {
            device: device_name,
            message,
        });
    }

    let samples = collected.lock().map(|b| b.clone()).unwrap_or_default();

    if samples.is_empty() {
        return Err(CaptureError::Silent(device_name));
    }

    Ok(Capture {
        samples,
        sample_rate,
        device_name,
    })
}

/// Write a capture to a WAV file, so it can be kept and re-analysed later.
pub fn write_wav(capture: &Capture, path: &std::path::Path) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: capture.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in &capture.samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}
