//! Measurement of the microphone signal.
//!
//! # What this crate does and does not do
//!
//! Everything here is **offline**. Audio is captured into a buffer, the capture
//! stops, and then the buffer is analysed. Nothing runs inside a real-time audio
//! callback, which is what keeps this crate free of the xrun, lock-free-buffer
//! and clock-drift problems that dominate real-time audio work.
//!
//! # What the signal actually is
//!
//! The GoXLR exposes its mic to Windows *after* its onboard DSP. There is no
//! pre-DSP tap. So a measurement describes the mic **as currently processed**,
//! not the raw capsule.
//!
//! That is a constraint worth stating plainly rather than papering over, and it
//! shapes the tuner's design: it cannot compute correct settings in one shot from
//! a single measurement. It measures, adjusts, and measures again -- a closed
//! loop, converging -- which is also why every write must be verified. A write
//! that silently did not land would make the loop chase a signal that never moved.

pub mod capture;
pub mod measure;

pub use capture::{Capture, CaptureError};
pub use measure::Measurement;
