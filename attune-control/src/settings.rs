//! The mic settings surface the tuner reads and writes.
//!
//! These map onto the GoXLR's onboard DSP. Attune derives values here from
//! measurement and sends them to the device; the device does the processing.
//!
//! Every write goes through a read-back check. A setting the device silently
//! declines must not look like a success, because the tuner's A/B loop would
//! then learn from a change that never happened.

/// A single coherent mic configuration.
///
/// Implemented in the "Add write path and verify hardware round-trip" work order.
#[derive(Debug, Clone, PartialEq)]
pub struct MicSettings {
    /// Noise gate threshold in dB. The operator's Shure MV7 runs over XLR with
    /// its onboard DSP bypassed, and its low XLR output needs high preamp gain,
    /// which raises the noise floor -- so this value carries more weight here
    /// than it would on a hotter mic.
    pub gate_threshold_db: i8,

    /// Compressor threshold in dB.
    pub compressor_threshold_db: i8,

    /// Compressor ratio.
    pub compressor_ratio: u8,
}
