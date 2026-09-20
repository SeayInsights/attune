//! Headphone correction for the GoXLR's output buses.
//!
//! # The capability this exists for
//!
//! The GoXLR exposes Game, Music, Chat and System as **separate Windows
//! endpoints**, because it splits them in hardware before Windows ever mixes
//! them. That means a different correction can run on each one at the same time
//! -- a competitive voicing on Game while Music runs something for listening.
//!
//! Tools that equalise the system output cannot do this, and tools that
//! equalise one mix cannot either. It is available here only because of what the
//! hardware already does.
//!
//! # Where the processing happens
//!
//! Not here. This crate decides *what* the correction should be and writes it
//! where Equalizer APO will find it; APO does the filtering, inside the Windows
//! audio pipeline. Attune adds no latency because Attune is not in the path.
//!
//! The cost of that is APO's own limit: audio bypassing the Windows effect
//! infrastructure, such as WASAPI exclusive mode or ASIO, is untouched.
//!
//! # Correction versus voicing
//!
//! [`profiles`] explains the split, which is the main idea worth understanding
//! before using any of this: a measured headphone correction and an opinionated
//! voicing are different things, and they compose.

pub mod apo;
pub mod attachment;
pub mod autoeq;
pub mod curve;
pub mod headroom;
pub mod profiles;
#[cfg(windows)]
pub mod spatial;

pub use curve::{Curve, Filter, FilterKind};
pub use profiles::Voicing;

/// A ready-to-apply correction for one bus.
#[derive(Debug, Clone)]
pub struct BusSetup {
    /// The Windows endpoint name.
    pub device: String,
    /// Optional measured headphone correction, as imported from AutoEQ.
    pub correction: Option<Curve>,
    /// The voicing to apply on top.
    pub voicing: Voicing,
}

/// Build the final, headroom-managed curve for a bus.
pub fn build(setup: &BusSetup) -> headroom::Managed {
    let composed = profiles::compose(setup.correction.as_ref(), setup.voicing);
    headroom::manage(&composed, headroom::DEFAULT_MARGIN_DB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_bus_never_clips() {
        let setup = BusSetup {
            device: "Music (TC-HELICON GoXLR)".into(),
            correction: Some(Curve {
                name: "DT 990 Pro".into(),
                preamp_db: 0.0,
                // AutoEQ curves for this model boost bass toward Harman, which
                // is exactly the case that needs headroom.
                filters: vec![Filter {
                    kind: FilterKind::LowShelf,
                    freq_hz: 105.0,
                    gain_db: 6.0,
                    q: 0.7,
                }],
            }),
            voicing: profiles::MUSIC,
        };

        let managed = build(&setup);
        let mut hz = 20.0_f32;
        while hz < 20_000.0 {
            assert!(managed.curve.response_at(hz) <= 0.01, "clips at {hz:.0} Hz");
            hz *= crate::curve::TWELFTH_OCTAVE;
        }
    }

    /// Different buses, different curves, at the same time. The whole point.
    #[test]
    fn different_buses_can_carry_different_curves_simultaneously() {
        let game = build(&BusSetup {
            device: "Game (TC-HELICON GoXLR)".into(),
            correction: None,
            voicing: profiles::COMPETITIVE,
        });
        let music = build(&BusSetup {
            device: "Music (TC-HELICON GoXLR)".into(),
            correction: None,
            voicing: profiles::MUSIC,
        });

        assert!(
            game.curve.response_at(60.0) < music.curve.response_at(60.0) - 4.0,
            "the competitive bus should have markedly less bass than the music bus"
        );

        let config = apo::render(&[
            apo::BusCurve::from_composite("Game (TC-HELICON GoXLR)", game.curve),
            apo::BusCurve::from_composite("Music (TC-HELICON GoXLR)", music.curve),
        ]);
        assert_eq!(config.matches("Device:").count(), 2);
    }
}
