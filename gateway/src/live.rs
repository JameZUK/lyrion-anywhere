//! Lower-data live streams (docs/plans/streaming-quality.md): a FLAC stream from LMS
//! re-encoded to Ogg Opus on its way to the player, for clients on mobile data.
//!
//! - **What:** only FLAC (`strm` format `f`); the session rewrites the `strm` it passes on to
//!   say Ogg Opus (`u`), and the player's core probes the content.
//! - **How:** LMS's bytes go to `ffmpeg`'s standard input by a feeder task; its standard
//!   output is the stream the relay counts, keeps for replay and serves, so `?offset=`,
//!   acknowledgements and resume work unchanged on the Opus stream. The encoder is libopus,
//!   or ffmpeg's own when that is all it has ([`crate::transcode::OpusEncoder`]).
//! - **Back-pressure:** a relay that stops reading stops `ffmpeg` (its output pipe fills),
//!   which stops reading its input, which stops the feeder reading LMS. At most the pipes'
//!   buffers are read ahead.
//! - **Hardening** (as for downloads, [`crate::transcode`]): a fixed argument list with no
//!   shell and an empty environment, the input demuxer forced to FLAC, only the `pipe`
//!   protocol allowed, no metadata copied, one thread. The process is killed and the
//!   feeder stopped when the stream is dropped (a session ending, a newer stream, a stall).
//! - **Failures:** a failed upstream or a failed `ffmpeg` (a corrupt or hostile stream) ends
//!   the stream with an error, never cleanly, so the player can't take it for the end of the
//!   track.
//! - **LMS's byte counts:** see [`Counts::upstream_bytes`].

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::OwnedSemaphorePermit;
use tokio::task::JoinHandle;

use crate::transcode::OpusEncoder;

/// Read size from `ffmpeg`'s output.
const CHUNK: usize = 64 * 1024;

/// What a client asks for (`"original"`, `"high"` or `"low"`, the "Data saver" level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// The stream as LMS sends it.
    #[default]
    Original,
    /// Opus 160 kb/s.
    High,
    /// Opus 96 kb/s ("Data saver").
    Low,
}

impl Quality {
    pub fn parse(s: &str) -> Option<Quality> {
        Some(match s {
            "original" => Quality::Original,
            "high" => Quality::High,
            "low" => Quality::Low,
            _ => return None,
        })
    }

    /// The Opus bit rate, kb/s (`None`: send the original).
    pub fn bitrate_k(self) -> Option<u32> {
        match self {
            Quality::Original => None,
            Quality::High => Some(160),
            Quality::Low => Some(96),
        }
    }

    pub const NAMES: [&'static str; 3] = ["original", "high", "low"];

    /// The level's name, as a client gives it.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Original => "original",
            Quality::High => "high",
            Quality::Low => "low",
        }
    }
}

/// The `strm` format code for Ogg Opus.
pub const OPUS_FORMAT: u8 = b'u';
/// Offset of the format code in a `strm` payload, followed by the four PCM parameters.
pub const STRM_FORMAT: usize = 2;

/// The `strm s` payload a session passes on when it re-encodes: the format becomes Ogg Opus
/// and the PCM parameters "unknown" (the player probes the content).
pub fn rewrite_strm(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < anywhere_core::proto::Strm::FIXED_LEN {
        return None;
    }
    let mut out = payload.to_vec();
    out[STRM_FORMAT] = OPUS_FORMAT;
    out[STRM_FORMAT + 1..STRM_FORMAT + 5].fill(b'?');
    Some(out)
}

/// The `ffmpeg` a live stream is encoded with: its path and the Opus encoder it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoder {
    pub path: PathBuf,
    pub opus: OpusEncoder,
}

/// `ffmpeg`'s argument list for a live encode at `bitrate_k` kb/s with `opus` (no shell is
/// involved).
pub fn args(bitrate_k: u32, opus: OpusEncoder) -> Vec<OsString> {
    let bitrate = format!("{}k", bitrate_k.clamp(32, 256));
    let mut a: Vec<&str> = vec![
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        "pipe",
        "-f",
        "flac",
        "-i",
        "pipe:0",
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-ac",
        "2",
        "-ar",
        "48000",
        "-c:a",
        opus.codec(),
    ];
    a.extend(opus.options());
    a.extend(["-b:a", bitrate.as_str()]);
    // libopus's own options; ffmpeg's built-in encoder has neither and would refuse them.
    if opus == OpusEncoder::Libopus {
        a.extend(["-vbr", "on", "-application", "audio"]);
    }
    a.extend([
        "-threads",
        "1",
        "-flush_packets",
        "1",
        // Deterministic output (rolling cache S3): no encoder or muxer version strings and
        // a fixed Ogg stream serial instead of a random one, so the same track at the same
        // level is the same bytes every time and a replay can be matched by its first bytes.
        "-fflags",
        "+bitexact",
        "-flags:a",
        "+bitexact",
        "-f",
        "ogg",
        "pipe:1",
    ]);
    a.into_iter().map(OsString::from).collect()
}

/// Bytes fed to `ffmpeg` and bytes it produced, for LMS's byte counts.
#[derive(Debug, Default)]
pub struct Counts {
    input: AtomicU64,
    output: AtomicU64,
    /// The upstream ended cleanly and `ffmpeg` produced everything.
    finished: AtomicBool,
}

impl Counts {
    /// The byte count LMS sent for `client` bytes of the Opus stream the player received:
    /// in proportion to what has been fed and produced so far, never more than was fed, and
    /// all of it once the player has the whole finished stream.
    pub fn upstream_bytes(&self, client: u64) -> u64 {
        let input = self.input.load(Ordering::Acquire);
        let output = self.output.load(Ordering::Acquire);
        if output == 0 {
            return 0;
        }
        if client >= output && self.finished.load(Ordering::Acquire) {
            return input;
        }
        let scaled = (client.min(output) as u128 * input as u128) / output as u128;
        (scaled as u64).min(input)
    }

    #[cfg(test)]
    pub(crate) fn set(&self, input: u64, output: u64, finished: bool) {
        self.input.store(input, Ordering::Release);
        self.output.store(output, Ordering::Release);
        self.finished.store(finished, Ordering::Release);
    }
}

/// Aborts the feeder when the stream is dropped.
struct Feeder(JoinHandle<Result<(), String>>);

impl Drop for Feeder {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Live {
    stdout: ChildStdout,
    child: Child,
    feeder: Option<Feeder>,
    counts: Arc<Counts>,
    done: bool,
    /// The live-encode slot, given back when the stream is dropped.
    _permit: Option<OwnedSemaphorePermit>,
}

/// Re-encode `upstream` (FLAC) to Ogg Opus at `bitrate_k` with `ffmpeg`. The stream ends
/// cleanly only when the upstream ended cleanly and `ffmpeg` succeeded.
pub fn encode(
    ffmpeg: &Encoder,
    bitrate_k: u32,
    upstream: BoxStream<'static, Result<Bytes, io::Error>>,
    permit: Option<OwnedSemaphorePermit>,
    counts: Arc<Counts>,
) -> io::Result<BoxStream<'static, Result<Bytes, io::Error>>> {
    let mut cmd = tokio::process::Command::from(crate::transcode::command(&ffmpeg.path));
    cmd.args(args(bitrate_k, ffmpeg.opus))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout"))?;
    let feeder = Feeder(tokio::spawn(feed(upstream, stdin, counts.clone())));
    let live = Live {
        stdout,
        child,
        feeder: Some(feeder),
        counts,
        done: false,
        _permit: permit,
    };
    Ok(futures_util::stream::unfold(live, |mut l| async move {
        if l.done {
            return None;
        }
        next(&mut l).await.map(|item| (item, l))
    })
    .boxed())
}

async fn feed(
    mut upstream: BoxStream<'static, Result<Bytes, io::Error>>,
    mut stdin: ChildStdin,
    counts: Arc<Counts>,
) -> Result<(), String> {
    while let Some(item) = upstream.next().await {
        let chunk = item.map_err(|e| format!("upstream: {e}"))?;
        stdin
            .write_all(&chunk)
            .await
            .map_err(|e| format!("encoder input: {e}"))?;
        counts.input.fetch_add(chunk.len() as u64, Ordering::AcqRel);
    }
    stdin
        .shutdown()
        .await
        .map_err(|e| format!("encoder input: {e}"))?;
    Ok(())
}

async fn next(l: &mut Live) -> Option<Result<Bytes, io::Error>> {
    let mut buf = vec![0u8; CHUNK];
    match l.stdout.read(&mut buf).await {
        Ok(0) => {
            // ffmpeg is done: a clean end only if the upstream ended cleanly and ffmpeg
            // succeeded.
            l.done = true;
            let fed = match l.feeder.take() {
                // Aborting a finished task (when `f` drops) does nothing.
                Some(mut f) => (&mut f.0)
                    .await
                    .unwrap_or_else(|_| Err("feeder stopped".into())),
                None => Err("feeder missing".into()),
            };
            let exit = l.child.wait().await;
            match (fed, exit) {
                (Ok(()), Ok(st)) if st.success() => {
                    l.counts.finished.store(true, Ordering::Release);
                    None
                }
                (Err(e), _) => Some(Err(io::Error::other(e))),
                _ => Some(Err(io::Error::other("the encoder failed"))),
            }
        }
        Ok(n) => {
            buf.truncate(n);
            l.counts.output.fetch_add(n as u64, Ordering::AcqRel);
            Some(Ok(Bytes::from(buf)))
        }
        Err(e) => {
            l.done = true;
            Some(Err(e))
        }
    }
}

/// Is this a stream to re-encode: FLAC, a level below Original, and `ffmpeg` with Opus?
pub fn wanted(format: u8, quality: Quality, opus_ffmpeg: bool) -> Option<u32> {
    if format != b'f' || !opus_ffmpeg {
        return None;
    }
    quality.bitrate_k()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_grammar() {
        assert_eq!(Quality::parse("original"), Some(Quality::Original));
        assert_eq!(Quality::parse("high"), Some(Quality::High));
        assert_eq!(Quality::parse("low"), Some(Quality::Low));
        for bad in [
            "",
            "High",
            "HIGH",
            " high",
            "lossless",
            "1",
            "low\0",
            "data saver",
        ] {
            assert_eq!(Quality::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Quality::High.bitrate_k(), Some(160));
        assert_eq!(Quality::Low.bitrate_k(), Some(96));
        assert_eq!(Quality::Original.bitrate_k(), None);
    }

    #[test]
    fn only_flac_is_re_encoded_and_only_when_possible() {
        assert_eq!(wanted(b'f', Quality::High, true), Some(160));
        assert_eq!(wanted(b'f', Quality::Low, true), Some(96));
        assert_eq!(wanted(b'f', Quality::Original, true), None);
        assert_eq!(wanted(b'f', Quality::High, false), None);
        for f in [b'm', b'o', b'u', b'a', b'l', b'p', b'?', 0] {
            assert_eq!(wanted(f, Quality::Low, true), None, "{}", f as char);
        }
    }

    #[test]
    fn strm_rewrite_says_opus_and_hides_pcm_parameters() {
        let mut p = vec![0u8; 24];
        p[..7].copy_from_slice(b"s1f2311");
        p.extend_from_slice(b"GET /stream.mp3?player=x HTTP/1.0\r\n\r\n");
        let out = rewrite_strm(&p).unwrap();
        assert_eq!(&out[..7], b"s1u????");
        assert_eq!(out[7..], p[7..]);
        assert_eq!(rewrite_strm(&p[..23]), None);
    }

    fn words(bitrate_k: u32, opus: OpusEncoder) -> Vec<String> {
        args(bitrate_k, opus)
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn args_are_fixed_and_hardened() {
        for opus in [OpusEncoder::Libopus, OpusEncoder::Native] {
            let a = words(96, opus);
            let pos = |w: &str| a.iter().position(|x| x == w).unwrap();
            assert!(pos("-protocol_whitelist") < pos("-i"));
            assert_eq!(a[pos("-protocol_whitelist") + 1], "pipe");
            assert_eq!(a[pos("-f") + 1], "flac"); // the input demuxer is forced
            assert!(pos("-f") < pos("-i"));
            assert_eq!(a[pos("-i") + 1], "pipe:0");
            assert_eq!(a.last().unwrap(), "pipe:1");
            assert_eq!(a[pos("-b:a") + 1], "96k");
            assert_eq!(a[pos("-map_metadata") + 1], "-1");
            assert_eq!(a[pos("-ar") + 1], "48000"); // all the built-in encoder takes
            // Deterministic output, as output options (after the input).
            assert_eq!(a[pos("-fflags") + 1], "+bitexact");
            assert_eq!(a[pos("-flags:a") + 1], "+bitexact");
            assert!(pos("-fflags") > pos("-i") && pos("-fflags") < pos("pipe:1"));
            assert!(!a.iter().any(|w| w.contains("http") || w.contains("file")));
            assert_eq!(a.iter().filter(|x| *x == "-i").count(), 1);
        }
        // Bit rates are clamped to something sane whatever is asked.
        assert!(words(100_000, OpusEncoder::Libopus).contains(&"256k".to_string()));
        assert!(words(0, OpusEncoder::Native).contains(&"32k".to_string()));
    }

    #[test]
    fn each_opus_encoder_gets_its_own_options() {
        let a = words(160, OpusEncoder::Libopus);
        let pos = |w: &str| a.iter().position(|x| x == w).unwrap();
        assert_eq!(a[pos("-c:a") + 1], "libopus");
        assert_eq!(a[pos("-vbr") + 1], "on");
        assert_eq!(a[pos("-application") + 1], "audio");
        assert!(!a.contains(&"-strict".to_string()));

        // ffmpeg's built-in encoder: experimental, so `-strict -2` as an output option right
        // after the codec, and none of libopus's private options, which it would refuse.
        let n = words(160, OpusEncoder::Native);
        let pos = |w: &str| n.iter().position(|x| x == w).unwrap();
        assert_eq!(n[pos("-c:a") + 1], "opus");
        assert_eq!(n[pos("-strict") + 1], "-2");
        assert!(pos("-i") < pos("-strict") && pos("-strict") < pos("pipe:1"));
        assert_eq!(n[pos("-b:a") + 1], "160k");
        assert!(!n.contains(&"-vbr".to_string()));
        assert!(!n.contains(&"-application".to_string()));
        assert!(!n.contains(&"libopus".to_string()));
    }

    #[test]
    fn lms_hears_the_bytes_it_sent() {
        let c = Counts::default();
        assert_eq!(c.upstream_bytes(1000), 0);
        c.set(10_000, 1_000, false);
        assert_eq!(c.upstream_bytes(0), 0);
        assert_eq!(c.upstream_bytes(500), 5_000);
        assert_eq!(c.upstream_bytes(1_000), 10_000);
        assert_eq!(c.upstream_bytes(u64::MAX), 10_000); // never more than was fed
        c.set(10_000, 1_000, true);
        c.set(12_345, 1_000, true);
        assert_eq!(c.upstream_bytes(1_000), 12_345);
        assert_eq!(c.upstream_bytes(999), 12_332);
        // Huge counts don't overflow.
        c.set(u64::MAX, u64::MAX / 2, false);
        assert_eq!(c.upstream_bytes(u64::MAX / 2), u64::MAX);
    }
}
