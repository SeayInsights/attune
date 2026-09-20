//! The mic settings surface the tuner reads and writes.
//!
//! These map onto the GoXLR's onboard DSP. Attune derives values from
//! measurement and sends them here; the device does the processing.
//!
//! Every write goes through a read-back check. A setting the device silently
//! declines must not look like a success.

use goxlr_ipc::{DaemonRequest, GoXLRCommand};
use goxlr_types::{CompressorRatio, MicrophoneType};

use crate::ControlError;
use crate::client::DaemonClient;

/// A snapshot of the mic chain, flattened out of the daemon's nested status.
#[derive(Debug, Clone, PartialEq)]
pub struct MicChain {
    /// Which preamp the device is using. An XLR dynamic mic is `Dynamic`; a
    /// phantom-powered condenser is `Condenser`; the 3.5 mm input is `Jack`.
    ///
    /// Typed rather than stringly, because every tuning decision downstream
    /// branches on it -- a condenser and a dynamic want very different gain.
    pub mic_type: MicrophoneType,
    /// Preamp gain in dB for the active mic type. Each type has its own stored
    /// gain on the device, so switching type does not carry gain across.
    pub gain_db: u16,

    /// Gate threshold in dB. Below this, the gate closes.
    pub gate_threshold_db: i8,
    /// How far the gate attenuates when closed, as a percentage.
    pub gate_attenuation: u8,
    /// Whether the gate is active at all.
    pub gate_enabled: bool,

    /// Compressor threshold in dB. At 0 the compressor never engages.
    pub compressor_threshold_db: i8,
    /// Compression ratio.
    pub compressor_ratio: CompressorRatio,
    /// Makeup gain applied after compression, in dB.
    pub compressor_makeup_db: i8,
}

impl DaemonClient {
    /// Read the current mic chain for a device.
    pub async fn mic_chain(&self, serial: &str) -> Result<MicChain, ControlError> {
        let mixer = self.mixer(serial).await?;
        let mic = &mixer.mic_status;

        let gain_db = mic.mic_gains[mic.mic_type];

        Ok(MicChain {
            mic_type: mic.mic_type,
            gain_db,
            gate_threshold_db: mic.noise_gate.threshold,
            gate_attenuation: mic.noise_gate.attenuation,
            gate_enabled: mic.noise_gate.enabled,
            compressor_threshold_db: mic.compressor.threshold,
            compressor_ratio: mic.compressor.ratio,
            compressor_makeup_db: mic.compressor.makeup_gain,
        })
    }

    /// Send a command to a device without checking that it landed.
    ///
    /// Prefer the verified setters below. This exists for commands whose effect
    /// is not readable back, where a verified wrapper would be a lie.
    pub async fn command(&self, serial: &str, cmd: GoXLRCommand) -> Result<(), ControlError> {
        self.send(DaemonRequest::Command(serial.to_string(), cmd))
            .await
            .map(|_| ())
    }

    /// Set the gate threshold, then confirm the device actually took it.
    pub async fn set_gate_threshold(
        &self,
        serial: &str,
        threshold_db: i8,
    ) -> Result<(), ControlError> {
        self.command(serial, GoXLRCommand::SetGateThreshold(threshold_db))
            .await?;

        let actual = self.mic_chain(serial).await?.gate_threshold_db;
        if actual != threshold_db {
            return Err(ControlError::WriteNotApplied {
                field: "gate_threshold_db".to_string(),
                requested: threshold_db.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(())
    }

    /// Set the compressor threshold, then confirm it.
    pub async fn set_compressor_threshold(
        &self,
        serial: &str,
        threshold_db: i8,
    ) -> Result<(), ControlError> {
        self.command(serial, GoXLRCommand::SetCompressorThreshold(threshold_db))
            .await?;

        let actual = self.mic_chain(serial).await?.compressor_threshold_db;
        if actual != threshold_db {
            return Err(ControlError::WriteNotApplied {
                field: "compressor_threshold_db".to_string(),
                requested: threshold_db.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(())
    }

    /// Set the preamp gain for the currently active mic type, then confirm it.
    ///
    /// Gain is stored per mic type on the device, so this deliberately targets
    /// the active one. Writing gain for a type that is not selected would report
    /// success and change nothing audible, which is exactly the class of silent
    /// no-op the verification here exists to catch.
    pub async fn set_mic_gain(&self, serial: &str, gain_db: u16) -> Result<(), ControlError> {
        let mic_type = self.mic_chain(serial).await?.mic_type;

        self.command(serial, GoXLRCommand::SetMicrophoneGain(mic_type, gain_db))
            .await?;

        let actual = self.mic_chain(serial).await?.gain_db;
        if actual != gain_db {
            return Err(ControlError::WriteNotApplied {
                field: format!("gain_db[{mic_type:?}]"),
                requested: gain_db.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(())
    }

    /// Set the compressor ratio, then confirm it.
    pub async fn set_compressor_ratio(
        &self,
        serial: &str,
        ratio: CompressorRatio,
    ) -> Result<(), ControlError> {
        self.command(serial, GoXLRCommand::SetCompressorRatio(ratio))
            .await?;

        let actual = self.mic_chain(serial).await?.compressor_ratio;
        if actual != ratio {
            return Err(ControlError::WriteNotApplied {
                field: "compressor_ratio".to_string(),
                requested: format!("{ratio:?}"),
                actual: format!("{actual:?}"),
            });
        }
        Ok(())
    }
}
