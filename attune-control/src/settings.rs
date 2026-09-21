//! The mic settings surface the tuner reads and writes.
//!
//! These map onto the GoXLR's onboard DSP. Attune derives values from
//! measurement and sends them here; the device does the processing.
//!
//! Every write goes through a read-back check. A setting the device silently
//! declines must not look like a success.

use goxlr_ipc::{DaemonRequest, GoXLRCommand};
use goxlr_types::{CompressorRatio, EqFrequencies, MicrophoneType, MiniEqFrequencies};

use crate::ControlError;
use crate::client::DaemonClient;

/// A snapshot of the mic chain, flattened out of the daemon's nested status.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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

    /// The mic equaliser, as (centre frequency in Hz, gain in dB), low to high.
    ///
    /// The full GoXLR has ten bands and the Mini six, so this is a list rather
    /// than a fixed array -- code that iterates it works on either without
    /// asking which device it is talking to.
    pub eq: Vec<EqBand>,
}

/// One equaliser band as the device currently has it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct EqBand {
    pub centre_hz: f32,
    pub gain_db: i8,
    /// Which band this is, on whichever equaliser the device has.
    ///
    /// Carried rather than recovered from `centre_hz`, because the frequencies
    /// are themselves adjustable on the full device -- a band moved off its
    /// nominal centre would become unidentifiable, and the setter would write to
    /// the wrong one or refuse.
    pub key: EqBandKey,
}

/// Identifies a band on either equaliser, and therefore which command sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum EqBandKey {
    /// The full GoXLR's ten-band equaliser.
    Full(EqFrequencies),
    /// The Mini's six-band equaliser.
    Mini(MiniEqFrequencies),
}

impl DaemonClient {
    /// Read the current mic chain for a device.
    pub async fn mic_chain(&self, serial: &str) -> Result<MicChain, ControlError> {
        let mixer = self.mixer(serial).await?;
        let mic = &mixer.mic_status;

        let gain_db = mic.mic_gains[mic.mic_type];

        // A Mini reports an empty ten-band map and populates the six-band one.
        // Reading whichever is present avoids branching on device type here.
        let mut eq: Vec<EqBand> = if mic.equaliser.gain.is_empty() {
            mic.equaliser_mini
                .gain
                .iter()
                .map(|(freq, gain)| EqBand {
                    centre_hz: mic
                        .equaliser_mini
                        .frequency
                        .get(freq)
                        .copied()
                        .unwrap_or(0.0),
                    gain_db: *gain,
                    key: EqBandKey::Mini(*freq),
                })
                .collect()
        } else {
            mic.equaliser
                .gain
                .iter()
                .map(|(freq, gain)| EqBand {
                    centre_hz: mic.equaliser.frequency.get(freq).copied().unwrap_or(0.0),
                    gain_db: *gain,
                    key: EqBandKey::Full(*freq),
                })
                .collect()
        };

        // HashMap iteration order is arbitrary; everything downstream assumes
        // ascending frequency.
        eq.sort_by(|a, b| {
            a.centre_hz
                .partial_cmp(&b.centre_hz)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(MicChain {
            mic_type: mic.mic_type,
            gain_db,
            gate_threshold_db: mic.noise_gate.threshold,
            gate_attenuation: mic.noise_gate.attenuation,
            gate_enabled: mic.noise_gate.enabled,
            compressor_threshold_db: mic.compressor.threshold,
            compressor_ratio: mic.compressor.ratio,
            compressor_makeup_db: mic.compressor.makeup_gain,
            eq,
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
    /// Set how hard the gate ducks when it closes, as a percentage.
    ///
    /// 100 is the only value that actually silences the signal. Anything less
    /// is a duck, and at high preamp gain a duck leaves the room clearly
    /// audible -- which reads as "the gate is not working" rather than "the
    /// gate is set to attenuate by 85%".
    pub async fn set_gate_attenuation(
        &self,
        serial: &str,
        percent: u8,
    ) -> Result<(), ControlError> {
        self.command(serial, GoXLRCommand::SetGateAttenuation(percent))
            .await?;

        let actual = self.mic_chain(serial).await?.gate_attenuation;
        if actual != percent {
            return Err(ControlError::WriteNotApplied {
                field: "gate_attenuation".to_string(),
                requested: percent.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(())
    }

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

    /// Set one equaliser band's gain, then confirm the device took it.
    pub async fn set_eq_gain(
        &self,
        serial: &str,
        key: EqBandKey,
        gain_db: i8,
    ) -> Result<(), ControlError> {
        let command = match key {
            EqBandKey::Full(freq) => GoXLRCommand::SetEqGain(freq, gain_db),
            EqBandKey::Mini(freq) => GoXLRCommand::SetEqMiniGain(freq, gain_db),
        };

        self.command(serial, command).await?;

        let actual = self
            .mic_chain(serial)
            .await?
            .eq
            .into_iter()
            .find(|b| b.key == key)
            .map(|b| b.gain_db);

        match actual {
            Some(actual) if actual == gain_db => Ok(()),
            Some(actual) => Err(ControlError::WriteNotApplied {
                field: format!("eq[{key:?}]"),
                requested: gain_db.to_string(),
                actual: actual.to_string(),
            }),
            None => Err(ControlError::WriteNotApplied {
                field: format!("eq[{key:?}]"),
                requested: gain_db.to_string(),
                actual: "band not present on this device".to_string(),
            }),
        }
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

impl DaemonClient {
    /// Load a GoXLR profile by name, then confirm the device took it.
    ///
    /// The read-back is the same discipline as every other write here: the
    /// daemon accepting a command is not the same as the device being in the
    /// state the command asked for, and for profile switching the difference
    /// would be silent -- the wrong voicing, with nothing to say so.
    pub async fn load_profile(&self, serial: &str, profile: &str) -> Result<(), ControlError> {
        self.command(serial, GoXLRCommand::LoadProfile(profile.to_string(), true))
            .await?;

        let actual = self.mixer(serial).await?.profile_name;
        if !actual.eq_ignore_ascii_case(profile) {
            return Err(ControlError::WriteNotApplied {
                field: "profile".to_string(),
                requested: profile.to_string(),
                actual,
            });
        }
        Ok(())
    }
}
