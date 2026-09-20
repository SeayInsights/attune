//! Transport to the daemon.
//!
//! The upstream daemon exposes the same command surface over three transports:
//!
//! | Transport | Use |
//! |---|---|
//! | Named pipe (Windows) / Unix socket | Local control, `[u32 BE length][JSON]` framing |
//! | HTTP `POST /api/command` | Simple request/response, easiest to test against |
//! | WebSocket `/api/websocket` | Live state, pushed as JSON Patch diffs |
//!
//! Attune uses HTTP for one-shot commands and the WebSocket for anything that
//! needs to observe state changing, so the tuner can confirm a write landed
//! rather than assume it did.
//!
//! Request and response types come from [`goxlr_ipc`] rather than being
//! redefined here. Redefining them would silently drift from the daemon on any
//! upstream change, and the whole point of tracking upstream is to inherit fixes.

use crate::ControlError;

/// Default HTTP port the upstream daemon listens on.
pub const DEFAULT_HTTP_PORT: u16 = 14564;

/// A connection to the Attune daemon.
///
/// Implemented in the "Build typed daemon API client read path" work order.
pub struct DaemonClient {
    _base_url: String,
}

impl DaemonClient {
    /// Connect to a daemon on localhost.
    pub fn new(port: u16) -> Self {
        Self {
            _base_url: format!("http://127.0.0.1:{port}"),
        }
    }

    /// Fetch full device state.
    ///
    /// Returns [`ControlError::NoDevice`] when the daemon is running but no
    /// GoXLR is attached -- a distinct case from the daemon being unreachable,
    /// because the two need different things from the operator.
    pub async fn status(&self) -> Result<(), ControlError> {
        Err(ControlError::Unreachable(
            "read path not yet implemented".to_string(),
        ))
    }
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new(DEFAULT_HTTP_PORT)
    }
}
