//! Transport to the daemon.
//!
//! The daemon exposes the same command surface over a named pipe, over HTTP at
//! `POST /api/command`, and over a WebSocket at `/api/websocket` that pushes
//! state as JSON Patch diffs. Attune uses HTTP for request/response work; the
//! WebSocket is added when something needs to watch state change live.
//!
//! Request and response types come from [`goxlr_ipc`] rather than being
//! redefined here. Redefining them would drift silently on any upstream change,
//! and inheriting upstream fixes is the whole reason this is a fork.

use goxlr_ipc::{DaemonRequest, DaemonResponse, DaemonStatus, MixerStatus};

use crate::ControlError;

/// Default HTTP port the daemon listens on.
pub const DEFAULT_HTTP_PORT: u16 = 14564;

/// A connection to the Attune daemon.
pub struct DaemonClient {
    base_url: String,
    http: reqwest::Client,
}

impl DaemonClient {
    /// Connect to a daemon on localhost.
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{port}/api/command"),
            http: reqwest::Client::new(),
        }
    }

    /// Send a request and return the daemon's response.
    ///
    /// A transport failure and a daemon-reported error are different problems
    /// with different fixes, so they stay distinct rather than collapsing into
    /// one opaque failure.
    pub async fn send(&self, request: DaemonRequest) -> Result<DaemonResponse, ControlError> {
        let response = self
            .http
            .post(&self.base_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| ControlError::Unreachable(e.to_string()))?;

        let parsed: DaemonResponse = response
            .json()
            .await
            .map_err(|e| ControlError::Protocol(e.to_string()))?;

        match parsed {
            DaemonResponse::Error(message) => Err(ControlError::Daemon(message)),
            other => Ok(other),
        }
    }

    /// Fetch full daemon status.
    pub async fn status(&self) -> Result<DaemonStatus, ControlError> {
        match self.send(DaemonRequest::GetStatus).await? {
            DaemonResponse::Status(status) => Ok(status),
            other => Err(ControlError::Protocol(format!(
                "expected Status, got {other:?}"
            ))),
        }
    }

    /// Serial of the first attached device.
    ///
    /// Multiple GoXLRs on one machine are supported by the daemon. Attune does
    /// not handle that yet, so this is explicit about picking one rather than
    /// pretending the question does not exist.
    pub async fn first_serial(&self) -> Result<String, ControlError> {
        let status = self.status().await?;
        let mut serials: Vec<&String> = status.mixers.keys().collect();
        serials.sort();
        serials
            .first()
            .map(|s| (*s).clone())
            .ok_or(ControlError::NoDevice)
    }

    /// Full state for one device.
    pub async fn mixer(&self, serial: &str) -> Result<MixerStatus, ControlError> {
        let status = self.status().await?;
        status
            .mixers
            .get(serial)
            .cloned()
            .ok_or(ControlError::NoDevice)
    }
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new(DEFAULT_HTTP_PORT)
    }
}
