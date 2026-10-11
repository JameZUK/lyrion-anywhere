//! Parametric equaliser, and a parser for AutoEq's `ParametricEQ.txt` profiles.
//!
//! The text format is the Equalizer APO subset AutoEq publishes for headphone correction:
//!
//! ```text
//! Preamp: -6.4 dB
//! Filter 1: ON LSC Fc 105 Hz Gain 5.8 dB Q 0.70
//! Filter 2: ON PK Fc 152 Hz Gain -2.5 dB Q 0.81
//! Filter 3: ON HSC Fc 10000 Hz Gain -2.6 dB Q 0.70
//! ```
//!
//! Profiles are user-supplied files, so the parser treats them as hostile: input size,
//! line length, filter count and every value are bounded, numbers must be finite, and
//! anything unexpected is an error naming the line. It never panics (see the robustness
//! tests).

use super::biquad::{Biquad, Coeffs, FilterKind, FilterSpec};
use super::gain::Gain;

/// Largest profile accepted, bytes.
pub const MAX_PROFILE_BYTES: usize = 64 * 1024;
/// Longest line accepted, bytes.
pub const MAX_LINE_BYTES: usize = 512;
/// Most filters in one profile (ON or OFF).
pub const MAX_FILTERS: usize = 32;
/// Accepted ranges.
pub const PREAMP_RANGE_DB: (f64, f64) = (-30.0, 30.0);
pub const GAIN_RANGE_DB: (f64, f64) = (-30.0, 30.0);
pub const FREQ_RANGE_HZ: (f64, f64) = (1.0, 100_000.0);
pub const Q_RANGE: (f64, f64) = (0.01, 100.0);

/// An equaliser profile: a preamp and a list of filters.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct EqProfile {
    pub preamp_db: f64,
    pub filters: Vec<FilterSpec>,
}

impl EqProfile {
    /// True when the profile changes nothing.
    pub fn is_flat(&self) -> bool {
        self.preamp_db == 0.0
            && self.filters.iter().all(|f| {
                matches!(
                    f.kind,
                    FilterKind::Peaking | FilterKind::LowShelf | FilterKind::HighShelf
                ) && f.gain_db == 0.0
            })
    }

    /// Check a profile built in code (or deserialised) against the parser's limits.
    pub fn validate(&self) -> Result<(), String> {
        if self.filters.len() > MAX_FILTERS {
            return Err(format!("more than {MAX_FILTERS} filters"));
        }
        in_range(self.preamp_db, PREAMP_RANGE_DB, "preamp")?;
        for f in &self.filters {
            in_range(f.freq_hz, FREQ_RANGE_HZ, "frequency")?;
            in_range(f.gain_db, GAIN_RANGE_DB, "gain")?;
            in_range(f.q, Q_RANGE, "Q")?;
        }
        Ok(())
    }
}

fn in_range(v: f64, (lo, hi): (f64, f64), what: &str) -> Result<(), String> {
    if v.is_finite() && v >= lo && v <= hi {
        Ok(())
    } else {
        Err(format!("{what} out of range ({lo}..={hi})"))
    }
}

/// Why a profile was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// 1-based line number (0 for whole-file problems).
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse an AutoEq / Equalizer APO `ParametricEQ.txt` profile. Filters marked `OFF` are
/// validated and then dropped.
pub fn parse_autoeq(text: &str) -> Result<EqProfile, ParseError> {
    let err = |line: usize, m: &str| ParseError {
        line,
        message: m.to_string(),
    };
    if text.len() > MAX_PROFILE_BYTES {
        return Err(err(0, "profile too large"));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut profile = EqProfile::default();
    let mut seen_preamp = false;
    let mut count = 0usize;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        if raw.len() > MAX_LINE_BYTES {
            return Err(err(n, "line too long"));
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (head, rest) = line.split_once(':').ok_or_else(|| err(n, "expected ':'"))?;
        let head_lc = head.trim().to_ascii_lowercase();
        if head_lc == "preamp" {
            if seen_preamp {
                return Err(err(n, "more than one Preamp line"));
            }
            seen_preamp = true;
            let mut t = rest.split_whitespace().peekable();
            let v = number(t.next(), n, "preamp")?;
            unit(&mut t, "db");
            if t.next().is_some() {
                return Err(err(n, "unexpected text after preamp"));
            }
            in_range(v, PREAMP_RANGE_DB, "preamp").map_err(|m| err(n, &m))?;
            profile.preamp_db = v;
        } else if head_lc == "filter" || head_lc.starts_with("filter ") {
            if let Some(num) = head_lc.strip_prefix("filter ") {
                let num = num.trim();
                if num.is_empty() || num.len() > 4 || !num.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(err(n, "bad filter number"));
                }
            }
            count += 1;
            if count > MAX_FILTERS {
                return Err(err(n, "too many filters"));
            }
            if let Some(spec) = parse_filter(rest, n)? {
                profile.filters.push(spec);
            }
        } else {
            return Err(err(n, "unknown directive"));
        }
    }
    Ok(profile)
}

fn number(tok: Option<&str>, line: usize, what: &str) -> Result<f64, ParseError> {
    let bad = || ParseError {
        line,
        message: format!("bad {what} value"),
    };
    let t = tok.ok_or_else(bad)?;
    // Only plain decimal numbers: no "inf", "NaN", hex or exponents.
    if t.is_empty()
        || t.len() > 24
        || !t
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+')
    {
        return Err(bad());
    }
    let v: f64 = t.parse().map_err(|_| bad())?;
    if v.is_finite() { Ok(v) } else { Err(bad()) }
}

/// Consume an optional unit token (e.g. "dB", "Hz") if it is next.
fn unit<'a>(t: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>, want: &str) {
    if t.peek().is_some_and(|u| u.eq_ignore_ascii_case(want)) {
        t.next();
    }
}

fn parse_filter(rest: &str, n: usize) -> Result<Option<FilterSpec>, ParseError> {
    let err = |m: &str| ParseError {
        line: n,
        message: m.to_string(),
    };
    let mut t = rest.split_whitespace().peekable();
    let on = match t.next().map(|s| s.to_ascii_uppercase()) {
        Some(s) if s == "ON" => true,
        Some(s) if s == "OFF" => false,
        _ => return Err(err("expected ON or OFF")),
    };
    let ty = t.next().ok_or_else(|| err("missing filter type"))?;
    let kind = match ty.to_ascii_uppercase().as_str() {
        "PK" | "PEQ" => FilterKind::Peaking,
        "LS" | "LSC" | "LSQ" => FilterKind::LowShelf,
        "HS" | "HSC" | "HSQ" => FilterKind::HighShelf,
        "LP" | "LPQ" => FilterKind::LowPass,
        "HP" | "HPQ" => FilterKind::HighPass,
        "NO" => FilterKind::Notch,
        _ => return Err(err("unsupported filter type")),
    };
    let (mut fc, mut gain, mut q) = (None, None, None);
    while let Some(key) = t.next() {
        match key.to_ascii_lowercase().as_str() {
            "fc" if fc.is_none() => {
                fc = Some(number(t.next(), n, "frequency")?);
                unit(&mut t, "hz");
            }
            "gain" if gain.is_none() => {
                gain = Some(number(t.next(), n, "gain")?);
                unit(&mut t, "db");
            }
            "q" if q.is_none() => q = Some(number(t.next(), n, "Q")?),
            _ => return Err(err("unexpected or repeated filter field")),
        }
    }
    let freq_hz = fc.ok_or_else(|| err("missing Fc"))?;
    let has_gain = matches!(
        kind,
        FilterKind::Peaking | FilterKind::LowShelf | FilterKind::HighShelf
    );
    let gain_db = match (has_gain, gain) {
        (true, Some(g)) => g,
        (true, None) => return Err(err("missing Gain")),
        (false, Some(_)) => return Err(err("this filter type takes no Gain")),
        (false, None) => 0.0,
    };
    let q = match (kind, q) {
        (_, Some(q)) => q,
        (FilterKind::Peaking | FilterKind::Notch, None) => return Err(err("missing Q")),
        // Equalizer APO's default for shelves and passes: Butterworth-like.
        (_, None) => std::f64::consts::FRAC_1_SQRT_2,
    };
    in_range(freq_hz, FREQ_RANGE_HZ, "frequency").map_err(|m| err(&m))?;
    in_range(gain_db, GAIN_RANGE_DB, "gain").map_err(|m| err(&m))?;
    in_range(q, Q_RANGE, "Q").map_err(|m| err(&m))?;
    Ok(on.then_some(FilterSpec {
        kind,
        freq_hz,
        gain_db,
        q,
    }))
}

/// A running equaliser for interleaved audio.
#[derive(Debug, Clone)]
pub struct ParametricEq {
    profile: EqProfile,
    preamp: Gain,
    biquads: Vec<Biquad>,
    channels: usize,
    sample_rate: u32,
}

impl ParametricEq {
    /// Build an equaliser. The profile is validated; an invalid one is refused.
    pub fn new(profile: EqProfile, sample_rate: u32, channels: usize) -> Result<Self, String> {
        profile.validate()?;
        let channels = channels.max(1);
        let mut eq = ParametricEq {
            preamp: Gain::new(profile.preamp_db),
            biquads: profile
                .filters
                .iter()
                .map(|_| Biquad::new(Coeffs::IDENTITY, channels))
                .collect(),
            profile,
            channels,
            sample_rate: 0,
        };
        eq.set_sample_rate(sample_rate);
        Ok(eq)
    }

    pub fn profile(&self) -> &EqProfile {
        &self.profile
    }

    pub fn is_bypassed(&self) -> bool {
        self.profile.is_flat()
    }

    /// Recompute every filter for a new sample rate (state is cleared).
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        for (bq, spec) in self.biquads.iter_mut().zip(&self.profile.filters) {
            bq.coeffs = Coeffs::design(spec, f64::from(sample_rate));
            bq.reset();
        }
    }

    pub fn reset(&mut self) {
        for bq in &mut self.biquads {
            bq.reset();
        }
    }

    /// Combined magnitude response at `freq_hz`, dB (including the preamp).
    pub fn response_db(&self, freq_hz: f64) -> f64 {
        self.profile.preamp_db
            + self
                .biquads
                .iter()
                .map(|b| b.coeffs.response_db(freq_hz, f64::from(self.sample_rate)))
                .sum::<f64>()
    }

    pub fn process(&mut self, buf: &mut [f64]) {
        if self.is_bypassed() {
            return;
        }
        self.preamp.process(buf);
        for bq in &mut self.biquads {
            if bq.coeffs != Coeffs::IDENTITY {
                bq.process(buf);
            }
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Preamp: -6.4 dB\r\n\
Filter 1: ON LSC Fc 105 Hz Gain 5.8 dB Q 0.70\n\
Filter 2: ON PK Fc 152 Hz Gain -2.5 dB Q 0.81\n\
Filter 3: OFF PK Fc 1000 Hz Gain 3 dB Q 1\n\
Filter 4: ON HSC Fc 10000 Hz Gain -2.6 dB Q 0.70\n\
Filter 5: ON LP Fc 18000 Hz\n";

    #[test]
    fn parses_autoeq_profile() {
        let p = parse_autoeq(SAMPLE).unwrap();
        assert_eq!(p.preamp_db, -6.4);
        assert_eq!(p.filters.len(), 4, "OFF filter dropped");
        assert_eq!(p.filters[0].kind, FilterKind::LowShelf);
        assert_eq!(p.filters[1].gain_db, -2.5);
        assert_eq!(p.filters[2].kind, FilterKind::HighShelf);
        assert_eq!(p.filters[3].kind, FilterKind::LowPass);
        assert!((p.filters[3].q - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    }

    #[test]
    fn eq_response_matches_profile_at_any_rate() {
        let p = parse_autoeq("Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain 6 dB Q 1\n").unwrap();
        for rate in [44_100, 48_000, 96_000, 192_000] {
            let eq = ParametricEq::new(p.clone(), rate, 2).unwrap();
            assert!((eq.response_db(1000.0) - 3.0).abs() < 1e-9);
        }
    }

    #[test]
    fn refuses_hostile_profiles() {
        let bad = [
            "Preamp: inf dB",
            "Preamp: NaN dB",
            "Preamp: 1e400 dB",
            "Preamp: -99 dB",
            "Preamp: 1 dB\nPreamp: 2 dB",
            "Filter 1: ON PK Fc 0 Hz Gain 1 dB Q 1",
            "Filter 1: ON PK Fc 1000 Hz Gain 100 dB Q 1",
            "Filter 1: ON PK Fc 1000 Hz Gain 1 dB Q 0",
            "Filter 1: ON PK Fc 1000 Hz Gain 1 dB",
            "Filter 1: ON XX Fc 1000 Hz Gain 1 dB Q 1",
            "Filter 1: MAYBE PK Fc 1000 Hz Gain 1 dB Q 1",
            "Filter 1: ON PK Fc 1000 Hz Gain 1 dB Q 1 Q 2",
            "Filter 1: ON LP Fc 1000 Hz Gain 1 dB",
            "Filter x: ON PK Fc 1000 Hz Gain 1 dB Q 1",
            "Device: speakers",
            "garbage",
            "Filter 1: ON PK Fc 0x10 Hz Gain 1 dB Q 1",
        ];
        for b in bad {
            assert!(parse_autoeq(b).is_err(), "accepted {b:?}");
        }
        let many: String = (1..=MAX_FILTERS + 1)
            .map(|i| format!("Filter {i}: ON PK Fc 100 Hz Gain 1 dB Q 1\n"))
            .collect();
        assert_eq!(parse_autoeq(&many).unwrap_err().line, MAX_FILTERS + 1);
        assert!(parse_autoeq(&"#".repeat(MAX_PROFILE_BYTES + 1)).is_err());
        assert!(parse_autoeq(&format!("#{}", "x".repeat(MAX_LINE_BYTES))).is_err());
    }

    #[test]
    fn flat_profile_is_bit_transparent() {
        let p = parse_autoeq("Preamp: 0 dB\nFilter 1: ON PK Fc 1000 Hz Gain 0 dB Q 1\n").unwrap();
        let mut eq = ParametricEq::new(p, 44_100, 2).unwrap();
        assert!(eq.is_bypassed());
        let mut b = [0.1, 0.2, -0.3, 0.4];
        let o = b;
        eq.process(&mut b);
        assert_eq!(b, o);
    }

    #[test]
    fn filters_above_nyquist_do_nothing() {
        let p = parse_autoeq("Filter 1: ON PK Fc 30000 Hz Gain 6 dB Q 1\n").unwrap();
        let eq = ParametricEq::new(p, 44_100, 2).unwrap();
        assert!(eq.response_db(1000.0).abs() < 1e-12);
    }
}
