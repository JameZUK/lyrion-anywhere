//! Output format negotiation for bit-perfect USB DACs (Android 14+).
//!
//! Android can hand a USB DAC our stream untouched when the app sets *preferred mixer
//! attributes* with `MIXER_BEHAVIOR_BIT_PERFECT` for its media stream, and then plays in
//! exactly that format (encoding, sample rate, channel mask). Device makers may or may not
//! offer it, per DAC and per format, so it is decided at run time:
//!
//! 1. Before opening its output for a track, the engine asks the host's
//!    [`OutputNegotiator`] what the current output device offers ([`OutputOffers`]): the
//!    mixer attributes `AudioManager.getSupportedMixerAttributes` lists, or why there's
//!    nothing to offer (setting off, no USB DAC, Android older than 14).
//! 2. [`plan`] (a pure function) picks the offer for the source: stereo, bit-perfect, at
//!    the source's rate, in the shallowest integer format that holds the source's depth
//!    exactly (16-bit audio in a 24-bit container is lossless padding).
//! 3. The host applies it (`setPreferredMixerAttributes`); the engine then opens its
//!    AAudio stream in that integer format and checks it got it.
//!
//! The signal path claims bit-perfect end to end only when all of that happened and the
//! engine itself left the samples untouched (see [`super::signal`]). Every way it can fall
//! short has a reason ([`NotBitPerfect`]) for the panel.

/// Sample formats an output stream can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PcmEncoding {
    /// 16-bit signed integers.
    I16,
    /// 24-bit signed integers, packed in 3 bytes.
    I24Packed,
    /// 32-bit signed integers.
    I32,
    /// 32-bit float (Android's mixer path).
    Float,
}

impl PcmEncoding {
    /// Integer depth (`None` for float).
    pub fn bits(self) -> Option<u32> {
        match self {
            PcmEncoding::I16 => Some(16),
            PcmEncoding::I24Packed => Some(24),
            PcmEncoding::I32 => Some(32),
            PcmEncoding::Float => None,
        }
    }

    /// For the signal path, e.g. "24-bit" or "32-bit float".
    pub fn describe(self) -> String {
        match self.bits() {
            Some(b) => format!("{b}-bit integer"),
            None => "32-bit float".into(),
        }
    }
}

/// One mixer configuration the output device offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Record)]
pub struct MixerOffer {
    pub encoding: PcmEncoding,
    pub sample_rate: u32,
    pub channels: u32,
    /// `MIXER_BEHAVIOR_BIT_PERFECT` (otherwise the platform's default mixing).
    pub bit_perfect: bool,
}

/// Why the platform has nothing to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum OfferBlocker {
    /// The user turned bit-perfect USB output off.
    TurnedOff,
    /// The output isn't a USB audio device.
    NoUsbDac,
    /// Android 13 or older (bit-perfect mixer attributes are API 34).
    AndroidTooOld,
}

/// What the current output device offers.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutputOffers {
    /// The device's name (e.g. the DAC's product name); empty when unknown.
    pub device: String,
    pub blocker: Option<OfferBlocker>,
    /// Every mixer configuration listed for the device (bit-perfect or not).
    pub offers: Vec<MixerOffer>,
}

impl OutputOffers {
    /// Nothing to offer, for this reason.
    pub fn blocked(blocker: OfferBlocker) -> OutputOffers {
        OutputOffers {
            device: String::new(),
            blocker: Some(blocker),
            offers: Vec::new(),
        }
    }
}

/// Why the path isn't bit-perfect end to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotBitPerfect {
    /// Bit-perfect USB output is turned off in Settings.
    TurnedOff,
    /// The output isn't a USB DAC (speaker, headphones, Bluetooth).
    NoUsbDac,
    /// Android 13 or older.
    AndroidTooOld,
    /// The DAC offers no bit-perfect mode at all.
    NotOffered,
    /// The DAC offers bit-perfect, but not at the source's sample rate.
    RateNotOffered,
    /// The DAC's deepest bit-perfect format is shallower than the source (reduced, with
    /// dither).
    DacTooShallow,
    /// The source is deeper than 24 bits, more than the engine carries exactly.
    SourceTooDeep,
    /// Android refused the mixer attributes.
    NotApplied,
    /// The output stream couldn't be opened in the negotiated format.
    FormatRefused,
    /// A DSP stage (preamp, levelling, EQ, crossfeed, limiter) changes the samples.
    DspActive,
    /// The audio is resampled.
    Resampled,
    /// The source is lossy.
    LossySource,
    /// Something else changes samples: LMS volume or ReplayGain, ducking, a downmix.
    OtherProcessing,
    /// Crossfades between tracks are on (they mix tracks of different albums).
    Crossfade,
}

impl From<OfferBlocker> for NotBitPerfect {
    fn from(b: OfferBlocker) -> Self {
        match b {
            OfferBlocker::TurnedOff => NotBitPerfect::TurnedOff,
            OfferBlocker::NoUsbDac => NotBitPerfect::NoUsbDac,
            OfferBlocker::AndroidTooOld => NotBitPerfect::AndroidTooOld,
        }
    }
}

/// The host side of negotiation (Android: `AudioManager`). Called on the engine's thread,
/// before the output opens; keep it quick (binder calls are fine).
#[uniffi::export(with_foreign)]
pub trait OutputNegotiator: Send + Sync {
    /// What the current output device offers.
    fn offers(&self) -> OutputOffers;
    /// Use this offer for our media stream (`Some`), or go back to normal mixing (`None`).
    /// Returns whether the platform accepted it.
    fn apply(&self, offer: Option<MixerOffer>) -> bool;
}

/// The source, as far as negotiation cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpec {
    pub rate: u32,
    /// Integer depth of a lossless source (`None`: lossy or float).
    pub bits: Option<u32>,
}

/// What to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The mixer attributes to apply (`None`: normal mixing).
    pub offer: Option<MixerOffer>,
    /// The output stream's format.
    pub encoding: PcmEncoding,
    /// Processed audio is quantised (with dither) to this depth before the output; `None`
    /// for float output.
    pub quantise_bits: Option<u32>,
    /// Reasons, known already, why this won't be bit-perfect end to end.
    pub reasons: Vec<NotBitPerfect>,
}

impl Plan {
    /// Float through Android's mixer.
    pub fn mixed(reason: NotBitPerfect) -> Plan {
        Plan {
            offer: None,
            encoding: PcmEncoding::Float,
            quantise_bits: None,
            reasons: vec![reason],
        }
    }
}

/// The deepest integer depth the engine carries exactly (the core's `f32` PCM).
const EXACT_BITS: u32 = anywhere_core::dsp::pcm::F32_EXACT_BITS;

/// Choose the mixer attributes and stream format for `src` from what the device `offers`.
pub fn plan(src: SourceSpec, offers: &OutputOffers) -> Plan {
    if let Some(b) = offers.blocker {
        return Plan::mixed(b.into());
    }
    let bit_perfect: Vec<&MixerOffer> = offers
        .offers
        .iter()
        .filter(|o| o.bit_perfect && o.channels == 2 && o.encoding.bits().is_some())
        .collect();
    if bit_perfect.is_empty() {
        return Plan::mixed(NotBitPerfect::NotOffered);
    }
    let at_rate: Vec<&MixerOffer> = bit_perfect
        .into_iter()
        .filter(|o| o.sample_rate == src.rate)
        .collect();
    if at_rate.is_empty() {
        return Plan::mixed(NotBitPerfect::RateNotOffered);
    }
    // The depth the source needs: its own (rounded up to a container), or 24 bits for
    // lossy and float sources (dithered there).
    let need = match src.bits {
        Some(b) if b <= 16 => 16,
        Some(b) if b <= 24 => 24,
        Some(_) => 32,
        None => 24,
    };
    let depth = |o: &MixerOffer| o.encoding.bits().unwrap_or(0);
    let mut reasons = Vec::new();
    let chosen = match at_rate
        .iter()
        .filter(|o| depth(o) >= need)
        .min_by_key(|o| depth(o))
    {
        Some(o) => **o,
        None => {
            reasons.push(NotBitPerfect::DacTooShallow);
            **at_rate.iter().max_by_key(|o| depth(o)).expect("not empty")
        }
    };
    if src.bits.is_some_and(|b| b > EXACT_BITS) {
        reasons.push(NotBitPerfect::SourceTooDeep);
    }
    let bits = depth(&chosen);
    Plan {
        offer: Some(chosen),
        encoding: chosen.encoding,
        quantise_bits: Some(bits.min(EXACT_BITS)),
        reasons,
    }
}

/// The output as it was actually set up, for the signal path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteState {
    /// The mixer attributes were applied and the stream opened in their format and at the
    /// source's rate: Android passes our samples to the DAC untouched.
    pub direct: bool,
    pub encoding: PcmEncoding,
    /// The DAC's name, when known.
    pub device: String,
    /// Why the route itself isn't bit-perfect (empty when it is).
    pub reasons: Vec<NotBitPerfect>,
}

impl Default for RouteState {
    fn default() -> Self {
        RouteState {
            direct: false,
            encoding: PcmEncoding::Float,
            device: String::new(),
            reasons: vec![NotBitPerfect::NoUsbDac],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(encoding: PcmEncoding, rate: u32) -> MixerOffer {
        MixerOffer {
            encoding,
            sample_rate: rate,
            channels: 2,
            bit_perfect: true,
        }
    }

    fn dac(offers: Vec<MixerOffer>) -> OutputOffers {
        OutputOffers {
            device: "USB DAC".into(),
            blocker: None,
            offers,
        }
    }

    fn src(rate: u32, bits: Option<u32>) -> SourceSpec {
        SourceSpec { rate, bits }
    }

    /// A typical DAC: 16, 24 and 32-bit at the common rates.
    fn typical() -> OutputOffers {
        let mut v = Vec::new();
        for rate in [44_100, 48_000, 88_200, 96_000, 176_400, 192_000] {
            for e in [PcmEncoding::I16, PcmEncoding::I24Packed, PcmEncoding::I32] {
                v.push(offer(e, rate));
            }
            // A default-mixing float entry, as devices list too.
            v.push(MixerOffer {
                bit_perfect: false,
                ..offer(PcmEncoding::Float, rate)
            });
        }
        dac(v)
    }

    #[test]
    fn blockers_mean_float_through_the_mixer() {
        for (b, r) in [
            (OfferBlocker::TurnedOff, NotBitPerfect::TurnedOff),
            (OfferBlocker::NoUsbDac, NotBitPerfect::NoUsbDac),
            (OfferBlocker::AndroidTooOld, NotBitPerfect::AndroidTooOld),
        ] {
            let p = plan(src(44_100, Some(16)), &OutputOffers::blocked(b));
            assert_eq!(p, Plan::mixed(r));
            assert_eq!(p.encoding, PcmEncoding::Float);
            assert!(p.offer.is_none() && p.quantise_bits.is_none());
        }
    }

    #[test]
    fn the_source_depth_picks_the_shallowest_exact_container() {
        let o = typical();
        for rate in [44_100, 96_000, 192_000] {
            let p = plan(src(rate, Some(16)), &o);
            assert_eq!(p.encoding, PcmEncoding::I16);
            assert_eq!(p.offer, Some(offer(PcmEncoding::I16, rate)));
            assert_eq!(p.quantise_bits, Some(16));
            assert!(p.reasons.is_empty());
            let p = plan(src(rate, Some(24)), &o);
            assert_eq!(p.encoding, PcmEncoding::I24Packed);
            assert_eq!(p.quantise_bits, Some(24));
            assert!(p.reasons.is_empty());
        }
        // Odd depths round up to a container.
        assert_eq!(
            plan(src(48_000, Some(20)), &o).encoding,
            PcmEncoding::I24Packed
        );
        assert_eq!(plan(src(48_000, Some(8)), &o).encoding, PcmEncoding::I16);
        // Lossy or float sources get 24 bits (dithered there).
        let p = plan(src(44_100, None), &o);
        assert_eq!(p.encoding, PcmEncoding::I24Packed);
        assert!(p.reasons.is_empty());
    }

    #[test]
    fn a_deeper_container_is_lossless_padding() {
        // A DAC that only takes 32-bit: 16- and 24-bit sources are padded, still exact.
        let o = dac(vec![offer(PcmEncoding::I32, 44_100)]);
        let p = plan(src(44_100, Some(16)), &o);
        assert_eq!(p.encoding, PcmEncoding::I32);
        assert_eq!(p.quantise_bits, Some(24), "the f32 path's exact depth");
        assert!(p.reasons.is_empty());
        // 24-bit only: a 16-bit source is padded.
        let o = dac(vec![offer(PcmEncoding::I24Packed, 96_000)]);
        assert!(plan(src(96_000, Some(16)), &o).reasons.is_empty());
    }

    #[test]
    fn shortfalls_have_reasons() {
        // A 16-bit-only DAC with a 24-bit source: reduced (with dither), and said so.
        let o = dac(vec![offer(PcmEncoding::I16, 96_000)]);
        let p = plan(src(96_000, Some(24)), &o);
        assert_eq!(p.encoding, PcmEncoding::I16);
        assert_eq!(p.quantise_bits, Some(16));
        assert_eq!(p.reasons, vec![NotBitPerfect::DacTooShallow]);
        // A 32-bit source can't be carried exactly by the engine.
        let p = plan(src(44_100, Some(32)), &typical());
        assert_eq!(p.encoding, PcmEncoding::I32);
        assert_eq!(p.reasons, vec![NotBitPerfect::SourceTooDeep]);
        // No bit-perfect mode at this rate: the mixer, which may resample.
        let p = plan(src(352_800, Some(24)), &typical());
        assert_eq!(p, Plan::mixed(NotBitPerfect::RateNotOffered));
        // A device listing only default-mixing entries, or none.
        let only_default = dac(vec![MixerOffer {
            bit_perfect: false,
            ..offer(PcmEncoding::I16, 44_100)
        }]);
        assert_eq!(
            plan(src(44_100, Some(16)), &only_default),
            Plan::mixed(NotBitPerfect::NotOffered)
        );
        assert_eq!(
            plan(src(44_100, Some(16)), &dac(vec![])),
            Plan::mixed(NotBitPerfect::NotOffered)
        );
    }

    #[test]
    fn mono_and_float_offers_are_never_chosen() {
        let o = dac(vec![
            MixerOffer {
                channels: 1,
                ..offer(PcmEncoding::I16, 44_100)
            },
            offer(PcmEncoding::Float, 44_100),
        ]);
        assert_eq!(
            plan(src(44_100, Some(16)), &o),
            Plan::mixed(NotBitPerfect::NotOffered)
        );
    }

    /// Whatever the device lists (hostile or nonsense entries included), the plan is
    /// consistent: a chosen offer is in the list, bit-perfect, stereo, integer and at the
    /// source rate; the stream format is the offer's; the quantisation depth fits both.
    #[test]
    fn random_offer_lists_give_consistent_plans() {
        let encs = [
            PcmEncoding::I16,
            PcmEncoding::I24Packed,
            PcmEncoding::I32,
            PcmEncoding::Float,
        ];
        let rates = [0, 1, 44_100, 48_000, 96_000, 192_000, u32::MAX];
        let mut s = 0x1234_5678_9abc_def0u64;
        let mut rnd = |n: u64| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s % n
        };
        for _ in 0..5_000 {
            let n = rnd(12) as usize;
            let offers: Vec<MixerOffer> = (0..n)
                .map(|_| MixerOffer {
                    encoding: encs[rnd(4) as usize],
                    sample_rate: rates[rnd(7) as usize],
                    channels: rnd(4) as u32,
                    bit_perfect: rnd(3) != 0,
                })
                .collect();
            let o = dac(offers.clone());
            let bits = [None, Some(8), Some(16), Some(24), Some(32)][rnd(5) as usize];
            let rate = rates[rnd(7) as usize];
            let p = plan(src(rate, bits), &o);
            match p.offer {
                Some(c) => {
                    assert!(offers.contains(&c));
                    assert!(c.bit_perfect && c.channels == 2 && c.sample_rate == rate);
                    assert_eq!(p.encoding, c.encoding);
                    let depth = c.encoding.bits().unwrap();
                    assert_eq!(p.quantise_bits, Some(depth.min(24)));
                }
                None => {
                    assert_eq!(p.encoding, PcmEncoding::Float);
                    assert_eq!(p.reasons.len(), 1);
                }
            }
        }
    }
}
