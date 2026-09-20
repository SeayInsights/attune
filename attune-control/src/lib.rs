//! Typed control-plane client for the Attune daemon.
//!
//! This crate owns every read and write of GoXLR device state. Nothing else in
//! Attune talks to the daemon directly.
//!
//! The device performs all mic DSP in its own hardware. Attune *configures* that
//! DSP; it never processes the mic signal in software. Keeping the mic out of the
//! Windows audio path is what preserves the GoXLR's zero-latency hardware
//! monitoring, and it is the reason the tuner can measure and adjust without
//! introducing a real-time constraint.
//!
//! Writes are verified: every setting sent is read back and compared against the
//! requested value, so a silent no-op surfaces as an error rather than as a
//! success the tuner would then learn from.

pub mod client;
pub mod settings;

/// Errors surfaced by the control plane.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    /// The daemon could not be reached on any configured transport.
    #[error("could not reach the Attune daemon: {0}")]
    Unreachable(String),

    /// The daemon answered, but no GoXLR was attached.
    #[error("no GoXLR device is connected")]
    NoDevice,

    /// A write was accepted but reading it back returned a different value.
    #[error("write to {field} was not applied: requested {requested}, device reports {actual}")]
    WriteNotApplied {
        field: String,
        requested: String,
        actual: String,
    },
}
