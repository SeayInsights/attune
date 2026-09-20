//! Typed control-plane client for the Attune daemon.
//!
//! This crate owns every read and write of GoXLR device state. Nothing else in
//! Attune talks to the daemon directly.
//!
//! The device performs all mic DSP in its own hardware. Attune *configures* that
//! DSP; it never processes the mic signal in software. Keeping the mic out of the
//! Windows audio path is what preserves the GoXLR's zero-latency hardware
//! monitoring, and it is why the tuner can measure and adjust without taking on a
//! real-time constraint.
//!
//! Writes are verified: every setting sent is read back and compared against the
//! requested value, so a silent no-op surfaces as an error rather than as a
//! success the tuner would then learn from.

pub mod autoswitch;
pub mod client;
pub mod diagnose;
pub mod settings;

/// Errors surfaced by the control plane.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    /// The daemon could not be reached.
    #[error("could not reach the Attune daemon: {0}. Is it running?")]
    Unreachable(String),

    /// The daemon answered, but with an error of its own.
    #[error("daemon reported an error: {0}")]
    Daemon(String),

    /// The daemon answered with something we could not interpret.
    #[error("unexpected response from daemon: {0}")]
    Protocol(String),

    /// The daemon is running, but no GoXLR is attached.
    #[error("no GoXLR device is connected")]
    NoDevice,

    /// A write was accepted but reading it back returned a different value.
    ///
    /// This is the case the tuner must never mistake for success. Learning a
    /// preference from a change that did not happen poisons every later round.
    #[error("write to {field} was not applied: requested {requested}, device reports {actual}")]
    WriteNotApplied {
        field: String,
        requested: String,
        actual: String,
    },
}
