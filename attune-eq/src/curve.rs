//! Parametric correction curves.
//!
//! The format is the one AutoEQ exports and Equalizer APO consumes, which is
//! not a coincidence worth hiding: using it directly means the thousands of
//! existing headphone measurements load without translation, and an exported
//! curve can be pasted into any other tool that speaks it.

use std::fmt;

/// Frequency ratio of one twelfth of an octave, for sweeping a response.
///
/// Fine enough not to step over a narrow filter, cheap enough to run on every
/// recalculation.
pub const TWELFTH_OCTAVE: f32 = 1.059_463_1;

/// The shape of one filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FilterKind {
    /// Peaking. Lifts or cuts around a centre frequency.
    Peaking,
    /// Low shelf. Lifts or cuts everything below the corner.
    LowShelf,
    /// High shelf. Lifts or cuts everything above the corner.
    HighShelf,
}

impl FilterKind {
    /// The two-letter code used in AutoEQ and APO files.
    pub fn code(&self) -> &'static str {
        match self {
            FilterKind::Peaking => "PK",
            FilterKind::LowShelf => "LSC",
            FilterKind::HighShelf => "HSC",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        match code {
            "PK" | "PEQ" => Some(FilterKind::Peaking),
            "LS" | "LSC" | "LSQ" => Some(FilterKind::LowShelf),
            "HS" | "HSC" | "HSQ" => Some(FilterKind::HighShelf),
            _ => None,
        }
    }
}

/// One filter in a curve.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Filter {
    pub kind: FilterKind,
    /// Centre or corner frequency in Hz.
    pub freq_hz: f32,
    /// Gain in dB. Negative cuts.
    pub gain_db: f32,
    /// Q. Higher is narrower.
    pub q: f32,
}

impl Filter {
    /// This filter's contribution at a frequency, in dB.
    ///
    /// An approximation adequate for headroom accounting and for drawing the
    /// curve: it gives the right peak gain, the right corner behaviour and a
    /// plausible skirt. It is not the exact biquad magnitude response, and
    /// nothing here pretends otherwise -- the actual filtering is done by APO,
    /// not by this code.
    pub fn response_at(&self, hz: f32) -> f32 {
        if hz <= 0.0 || self.freq_hz <= 0.0 {
            return 0.0;
        }
        // Distance in octaves from the filter's frequency.
        let octaves = (hz / self.freq_hz).log2();

        match self.kind {
            FilterKind::Peaking => {
                // Bandwidth in octaves falls as Q rises.
                let bw = (1.0 / self.q.max(0.05)).max(0.05);
                let x = octaves / bw;
                self.gain_db * (-x * x).exp()
            }
            FilterKind::LowShelf => {
                // Full gain well below the corner, none well above it.
                self.gain_db * (1.0 / (1.0 + (octaves * 2.0).exp()))
            }
            FilterKind::HighShelf => self.gain_db * (1.0 / (1.0 + (-octaves * 2.0).exp())),
        }
    }
}

impl fmt::Display for Filter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ON {} Fc {:.0} Hz Gain {:.2} dB Q {:.4}",
            self.kind.code(),
            self.freq_hz,
            self.gain_db,
            self.q
        )
    }
}

/// A complete correction.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Curve {
    /// Where it came from, for display.
    pub name: String,
    /// Broadband gain applied before the filters, in dB.
    ///
    /// Boosting filters need headroom or they clip; the preamp is what buys it.
    pub preamp_db: f32,
    pub filters: Vec<Filter>,
}

/// Why a curve could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CurveError {
    #[error("no filters found -- is this an AutoEQ ParametricEQ file?")]
    Empty,
    #[error("line {line}: {message}")]
    BadLine { line: usize, message: String },
}

impl Curve {
    /// The curve's total response at a frequency, in dB, preamp included.
    pub fn response_at(&self, hz: f32) -> f32 {
        self.preamp_db + self.filters.iter().map(|f| f.response_at(hz)).sum::<f32>()
    }

    /// The highest total boost anywhere in the audible band, in dB.
    ///
    /// This is the number that decides how much headroom a curve needs. Summing
    /// the individual filter gains would overstate it badly, because filters at
    /// different frequencies do not add at the same place.
    pub fn peak_gain_db(&self) -> f32 {
        let mut peak: f32 = 0.0;
        // Twelfth-octave sweep: fine enough that a narrow filter is not stepped
        // over, cheap enough to run on every recalculation.
        let mut hz = 20.0_f32;
        while hz < 20_000.0 {
            peak = peak.max(self.filters.iter().map(|f| f.response_at(hz)).sum::<f32>());
            hz *= TWELFTH_OCTAVE;
        }
        peak
    }

    /// Parse the AutoEQ / Equalizer APO parametric format.
    ///
    /// Unknown lines are skipped rather than rejected. These files are shared
    /// between many tools and routinely carry comments and extra directives; a
    /// parser that refuses anything unfamiliar is one that fails on real files.
    pub fn parse(name: &str, text: &str) -> Result<Self, CurveError> {
        let mut curve = Curve {
            name: name.to_string(),
            ..Default::default()
        };

        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(rest) = line.strip_prefix("Preamp:") {
                curve.preamp_db = parse_number(rest).ok_or_else(|| CurveError::BadLine {
                    line: index + 1,
                    message: format!("could not read a preamp value from {rest:?}"),
                })?;
                continue;
            }

            if !line.starts_with("Filter") {
                continue;
            }

            // "Filter 1: ON PK Fc 105 Hz Gain 5.5 dB Q 0.70"
            let Some((_, spec)) = line.split_once(':') else {
                continue;
            };
            let tokens: Vec<&str> = spec.split_whitespace().collect();

            // A disabled filter is not an error, it is a filter that is off.
            if tokens.first().map(|t| t.eq_ignore_ascii_case("OFF")) == Some(true) {
                continue;
            }

            let Some(kind) = tokens.get(1).and_then(|t| FilterKind::from_code(t)) else {
                continue;
            };

            let freq = field(&tokens, "Fc");
            let gain = field(&tokens, "Gain");
            let q = field(&tokens, "Q");

            match (freq, gain) {
                (Some(freq_hz), Some(gain_db)) => curve.filters.push(Filter {
                    kind,
                    freq_hz,
                    gain_db,
                    // Shelves in some exports carry no Q. 0.707 is the usual
                    // default and the one AutoEQ assumes.
                    q: q.unwrap_or(0.707),
                }),
                _ => {
                    return Err(CurveError::BadLine {
                        line: index + 1,
                        message: "filter is missing Fc or Gain".to_string(),
                    });
                }
            }
        }

        if curve.filters.is_empty() {
            return Err(CurveError::Empty);
        }
        Ok(curve)
    }
}

/// The number following a named field, e.g. `Fc 105 Hz` -> 105.
fn field(tokens: &[&str], name: &str) -> Option<f32> {
    tokens
        .iter()
        .position(|t| t.eq_ignore_ascii_case(name))
        .and_then(|i| tokens.get(i + 1))
        .and_then(|v| v.parse::<f32>().ok())
}

/// First number in a string, tolerating a trailing unit.
fn parse_number(s: &str) -> Option<f32> {
    s.split_whitespace().next()?.parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real AutoEQ export, in the shape they actually ship.
    const AUTOEQ_SAMPLE: &str = "\
Preamp: -6.8 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
Filter 2: ON PK Fc 250 Hz Gain -2.1 dB Q 1.20
Filter 3: ON PK Fc 8000 Hz Gain -6.4 dB Q 3.00
Filter 4: OFF PK Fc 1000 Hz Gain 3.0 dB Q 1.00
";

    #[test]
    fn parses_an_autoeq_export() {
        let c = Curve::parse("DT 990", AUTOEQ_SAMPLE).unwrap();
        assert_eq!(c.preamp_db, -6.8);
        assert_eq!(c.filters.len(), 3, "the OFF filter should be skipped");
        assert_eq!(c.filters[0].kind, FilterKind::LowShelf);
        assert_eq!(c.filters[2].freq_hz, 8000.0);
        assert_eq!(c.filters[2].gain_db, -6.4);
    }

    #[test]
    fn comments_and_unknown_directives_do_not_break_parsing() {
        let text = format!("# exported by something\nDevice: Whatever\n{AUTOEQ_SAMPLE}");
        let c = Curve::parse("x", &text).unwrap();
        assert_eq!(c.filters.len(), 3);
    }

    #[test]
    fn a_file_with_no_filters_is_an_error_not_an_empty_curve() {
        assert!(matches!(
            Curve::parse("x", "Preamp: -3 dB\n"),
            Err(CurveError::Empty)
        ));
    }

    #[test]
    fn a_peaking_filter_peaks_at_its_own_frequency() {
        let f = Filter {
            kind: FilterKind::Peaking,
            freq_hz: 1000.0,
            gain_db: 6.0,
            q: 1.0,
        };
        assert!((f.response_at(1000.0) - 6.0).abs() < 0.01);
        assert!(f.response_at(1000.0) > f.response_at(4000.0));
        assert!(f.response_at(1000.0) > f.response_at(250.0));
    }

    #[test]
    fn a_low_shelf_acts_below_its_corner_and_not_above() {
        let f = Filter {
            kind: FilterKind::LowShelf,
            freq_hz: 200.0,
            gain_db: 6.0,
            q: 0.707,
        };
        assert!(f.response_at(30.0) > 5.0, "should apply well below");
        assert!(f.response_at(4000.0).abs() < 0.5, "should not apply above");
    }

    /// Peak gain is what decides headroom, and summing filter gains overstates
    /// it: filters at different frequencies do not add at the same place.
    #[test]
    fn peak_gain_is_measured_not_summed() {
        let c = Curve {
            name: "t".into(),
            preamp_db: 0.0,
            filters: vec![
                Filter {
                    kind: FilterKind::Peaking,
                    freq_hz: 100.0,
                    gain_db: 6.0,
                    q: 2.0,
                },
                Filter {
                    kind: FilterKind::Peaking,
                    freq_hz: 8000.0,
                    gain_db: 6.0,
                    q: 2.0,
                },
            ],
        };
        let peak = c.peak_gain_db();
        assert!(peak > 5.5, "should find the 6 dB peaks, got {peak}");
        assert!(peak < 9.0, "must not sum to 12 dB, got {peak}");
    }

    #[test]
    fn filters_round_trip_through_the_apo_line_format() {
        let f = Filter {
            kind: FilterKind::Peaking,
            freq_hz: 2500.0,
            gain_db: -3.25,
            q: std::f32::consts::SQRT_2,
        };
        let line = format!("Filter 1: {f}");
        let back = Curve::parse("x", &line).unwrap();
        assert_eq!(back.filters[0].freq_hz, 2500.0);
        assert!((back.filters[0].gain_db - -3.25).abs() < 0.01);
    }
}
