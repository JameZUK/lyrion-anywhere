//! Slimproto wire format.
//!
//! Framing differs by direction:
//! - server → client: `u16 length (BE)` covering opcode + payload, then `[u8; 4]` opcode, then payload.
//! - client → server: `[u8; 4]` opcode, `u32 length (BE)` of the payload, then payload.
//!
//! Field layouts follow the Lyrion Slimproto reference
//! (<https://lyrion.org/reference/slimproto-protocol/>). This is a clean-room implementation.

use std::fmt;

/// Device ID reported in HELO. 12 = squeezelite-class software player, which LMS
/// treats as a fully capable modern player (gapless, all codecs it advertises).
pub const DEVICE_ID_SOFTWARE: u8 = 12;

/// A raw frame: opcode and payload.
pub type RawFrame = ([u8; 4], Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    Truncated {
        opcode: [u8; 4],
        need: usize,
        got: usize,
    },
    FrameTooLarge(usize),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::Truncated { opcode, need, got } => write!(
                f,
                "{} payload truncated: need {need} bytes, got {got}",
                String::from_utf8_lossy(opcode)
            ),
            ProtoError::FrameTooLarge(n) => write!(f, "frame too large: {n} bytes"),
        }
    }
}

impl std::error::Error for ProtoError {}

// ---------------------------------------------------------------------------
// Server → client
// ---------------------------------------------------------------------------

/// Stream command (`strm`). 24 fixed bytes followed by an HTTP request string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strm {
    /// `s` start, `p` pause, `u` unpause, `q` stop, `t` status, `f` flush, `a` skip ahead.
    pub command: u8,
    /// `0` don't start, `1` start when threshold reached, `2`/`3` direct-stream variants.
    pub autostart: u8,
    /// `f` flac, `m` mp3, `a` aac, `o` ogg, `u` opus, `l` alac, `p` pcm, ...
    pub format: u8,
    pub pcm_sample_size: u8,
    pub pcm_sample_rate: u8,
    pub pcm_channels: u8,
    pub pcm_endianness: u8,
    /// Input buffer threshold in KiB before autostart.
    pub threshold: u8,
    pub spdif_enable: u8,
    pub transition_period: u8,
    pub transition_type: u8,
    pub flags: u8,
    pub output_threshold: u8,
    pub slaves: u8,
    /// Replay gain (16.16 fixed) for `s`; server timestamp for `t`;
    /// interval in ms for `p`/`a`; jiffies to unpause at for `u`.
    pub replay_gain: u32,
    pub server_port: u16,
    /// 0 means "the slimproto server's own address".
    pub server_ip: u32,
    /// HTTP request (request line + headers) to send to the stream server.
    pub request: Vec<u8>,
}

impl Strm {
    pub const FIXED_LEN: usize = 24;

    pub fn parse(p: &[u8]) -> Result<Self, ProtoError> {
        need(b"strm", p, Self::FIXED_LEN)?;
        Ok(Strm {
            command: p[0],
            autostart: p[1],
            format: p[2],
            pcm_sample_size: p[3],
            pcm_sample_rate: p[4],
            pcm_channels: p[5],
            pcm_endianness: p[6],
            threshold: p[7],
            spdif_enable: p[8],
            transition_period: p[9],
            transition_type: p[10],
            flags: p[11],
            output_threshold: p[12],
            slaves: p[13],
            replay_gain: be32(&p[14..18]),
            server_port: be16(&p[18..20]),
            server_ip: be32(&p[20..24]),
            request: p[24..].to_vec(),
        })
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::FIXED_LEN + self.request.len());
        v.extend_from_slice(&[
            self.command,
            self.autostart,
            self.format,
            self.pcm_sample_size,
            self.pcm_sample_rate,
            self.pcm_channels,
            self.pcm_endianness,
            self.threshold,
            self.spdif_enable,
            self.transition_period,
            self.transition_type,
            self.flags,
            self.output_threshold,
            self.slaves,
        ]);
        v.extend_from_slice(&self.replay_gain.to_be_bytes());
        v.extend_from_slice(&self.server_port.to_be_bytes());
        v.extend_from_slice(&self.server_ip.to_be_bytes());
        v.extend_from_slice(&self.request);
        v
    }
}

/// Volume (`audg`). Gains are 16.16 fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Audg {
    pub old_left: u32,
    pub old_right: u32,
    pub adjust: u8,
    pub preamp: u8,
    pub new_left: u32,
    pub new_right: u32,
}

impl Audg {
    fn parse(p: &[u8]) -> Result<Self, ProtoError> {
        need(b"audg", p, 18)?;
        Ok(Audg {
            old_left: be32(&p[0..4]),
            old_right: be32(&p[4..8]),
            adjust: p[8],
            preamp: p[9],
            new_left: be32(&p[10..14]),
            new_right: be32(&p[14..18]),
        })
    }

    /// Linear gain for each channel. When `adjust` is 0 LMS expects fixed (unity) volume.
    pub fn gains(&self) -> (f32, f32) {
        if self.adjust == 0 {
            return (1.0, 1.0);
        }
        (
            self.new_left as f32 / 65536.0,
            self.new_right as f32 / 65536.0,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Strm(Strm),
    Audg(Audg),
    /// Audio enable: (spdif, dac).
    Aude {
        spdif: bool,
        dac: bool,
    },
    /// Player setting. id 0 = player name: empty data means "tell me your name".
    Setd {
        id: u8,
        data: Vec<u8>,
    },
    /// Switch to another server. Always refused by Lyrion Anywhere clients.
    Serv {
        ip: u32,
        sync_group: Vec<u8>,
    },
    Vers(String),
    Other {
        opcode: [u8; 4],
        payload: Vec<u8>,
    },
}

impl ServerMsg {
    pub fn parse(opcode: [u8; 4], p: &[u8]) -> Result<Self, ProtoError> {
        Ok(match &opcode {
            b"strm" => ServerMsg::Strm(Strm::parse(p)?),
            b"audg" => ServerMsg::Audg(Audg::parse(p)?),
            b"aude" => {
                need(b"aude", p, 2)?;
                ServerMsg::Aude {
                    spdif: p[0] != 0,
                    dac: p[1] != 0,
                }
            }
            b"setd" => {
                need(b"setd", p, 1)?;
                ServerMsg::Setd {
                    id: p[0],
                    data: p[1..].to_vec(),
                }
            }
            b"serv" => {
                need(b"serv", p, 4)?;
                ServerMsg::Serv {
                    ip: be32(&p[0..4]),
                    sync_group: p[4..].to_vec(),
                }
            }
            b"vers" => ServerMsg::Vers(String::from_utf8_lossy(p).into_owned()),
            _ => ServerMsg::Other {
                opcode,
                payload: p.to_vec(),
            },
        })
    }

    pub fn opcode(&self) -> [u8; 4] {
        match self {
            ServerMsg::Strm(_) => *b"strm",
            ServerMsg::Audg(_) => *b"audg",
            ServerMsg::Aude { .. } => *b"aude",
            ServerMsg::Setd { .. } => *b"setd",
            ServerMsg::Serv { .. } => *b"serv",
            ServerMsg::Vers(_) => *b"vers",
            ServerMsg::Other { opcode, .. } => *opcode,
        }
    }
}

/// Incremental decoder for the server → client byte stream.
#[derive(Default)]
pub struct ServerFrameReader {
    buf: Vec<u8>,
}

impl ServerFrameReader {
    /// Largest frame accepted. Real server frames are a few KiB at most (`strm` carries an
    /// HTTP request); the u16 length field alone would allow 64 KiB, so cap it explicitly.
    pub const MAX_FRAME: usize = 32 * 1024;

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Returns the next complete frame as (opcode, payload), if any.
    pub fn next_frame(&mut self) -> Result<Option<RawFrame>, ProtoError> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        let len = be16(&self.buf[0..2]) as usize;
        if len > Self::MAX_FRAME {
            return Err(ProtoError::FrameTooLarge(len));
        }
        if len < 4 {
            // Malformed: drop the length prefix and whatever it claimed.
            let drop = (2 + len).min(self.buf.len());
            self.buf.drain(..drop);
            return Ok(None);
        }
        if self.buf.len() < 2 + len {
            return Ok(None);
        }
        let opcode = [self.buf[2], self.buf[3], self.buf[4], self.buf[5]];
        let payload = self.buf[6..2 + len].to_vec();
        self.buf.drain(..2 + len);
        Ok(Some((opcode, payload)))
    }

    pub fn next_msg(&mut self) -> Result<Option<ServerMsg>, ProtoError> {
        match self.next_frame()? {
            Some((op, p)) => ServerMsg::parse(op, &p).map(Some),
            None => Ok(None),
        }
    }
}

/// Encode a server → client frame (used by tests and the gateway's test harness).
pub fn encode_server_frame(opcode: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(6 + payload.len());
    v.extend_from_slice(&((4 + payload.len()) as u16).to_be_bytes());
    v.extend_from_slice(&opcode);
    v.extend_from_slice(payload);
    v
}

// ---------------------------------------------------------------------------
// Client → server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helo {
    pub device_id: u8,
    pub revision: u8,
    pub mac: [u8; 6],
    pub uuid: [u8; 16],
    pub wlan_channels: u16,
    pub bytes_received: u64,
    pub lang: [u8; 2],
    /// Comma-separated capabilities, e.g. `Model=...,MaxSampleRate=...,flc,mp3`.
    pub capabilities: String,
}

impl Helo {
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(36 + self.capabilities.len());
        v.push(self.device_id);
        v.push(self.revision);
        v.extend_from_slice(&self.mac);
        v.extend_from_slice(&self.uuid);
        v.extend_from_slice(&self.wlan_channels.to_be_bytes());
        v.extend_from_slice(&self.bytes_received.to_be_bytes());
        v.extend_from_slice(&self.lang);
        v.extend_from_slice(self.capabilities.as_bytes());
        v
    }

    pub fn parse(p: &[u8]) -> Result<Self, ProtoError> {
        need(b"HELO", p, 36)?;
        Ok(Helo {
            device_id: p[0],
            revision: p[1],
            mac: p[2..8].try_into().unwrap(),
            uuid: p[8..24].try_into().unwrap(),
            wlan_channels: be16(&p[24..26]),
            bytes_received: u64::from_be_bytes(p[26..34].try_into().unwrap()),
            lang: [p[34], p[35]],
            capabilities: String::from_utf8_lossy(&p[36..]).into_owned(),
        })
    }
}

/// Status report (`STAT`). 53 bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stat {
    pub event: [u8; 4],
    pub num_crlf: u8,
    pub mas_initialized: u8,
    pub mas_mode: u8,
    pub stream_buffer_size: u32,
    pub stream_buffer_fullness: u32,
    pub bytes_received: u64,
    pub signal_strength: u16,
    pub jiffies: u32,
    pub output_buffer_size: u32,
    pub output_buffer_fullness: u32,
    pub elapsed_seconds: u32,
    pub voltage: u16,
    pub elapsed_milliseconds: u32,
    pub server_timestamp: u32,
    pub error_code: u16,
}

impl Stat {
    pub const LEN: usize = 53;

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::LEN);
        v.extend_from_slice(&self.event);
        v.push(self.num_crlf);
        v.push(self.mas_initialized);
        v.push(self.mas_mode);
        v.extend_from_slice(&self.stream_buffer_size.to_be_bytes());
        v.extend_from_slice(&self.stream_buffer_fullness.to_be_bytes());
        v.extend_from_slice(&self.bytes_received.to_be_bytes());
        v.extend_from_slice(&self.signal_strength.to_be_bytes());
        v.extend_from_slice(&self.jiffies.to_be_bytes());
        v.extend_from_slice(&self.output_buffer_size.to_be_bytes());
        v.extend_from_slice(&self.output_buffer_fullness.to_be_bytes());
        v.extend_from_slice(&self.elapsed_seconds.to_be_bytes());
        v.extend_from_slice(&self.voltage.to_be_bytes());
        v.extend_from_slice(&self.elapsed_milliseconds.to_be_bytes());
        v.extend_from_slice(&self.server_timestamp.to_be_bytes());
        v.extend_from_slice(&self.error_code.to_be_bytes());
        debug_assert_eq!(v.len(), Self::LEN);
        v
    }

    pub fn parse(p: &[u8]) -> Result<Self, ProtoError> {
        need(b"STAT", p, Self::LEN)?;
        Ok(Stat {
            event: p[0..4].try_into().unwrap(),
            num_crlf: p[4],
            mas_initialized: p[5],
            mas_mode: p[6],
            stream_buffer_size: be32(&p[7..11]),
            stream_buffer_fullness: be32(&p[11..15]),
            bytes_received: u64::from_be_bytes(p[15..23].try_into().unwrap()),
            signal_strength: be16(&p[23..25]),
            jiffies: be32(&p[25..29]),
            output_buffer_size: be32(&p[29..33]),
            output_buffer_fullness: be32(&p[33..37]),
            elapsed_seconds: be32(&p[37..41]),
            voltage: be16(&p[41..43]),
            elapsed_milliseconds: be32(&p[43..47]),
            server_timestamp: be32(&p[47..51]),
            error_code: be16(&p[51..53]),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Helo(Helo),
    Stat(Stat),
    /// HTTP response headers from the stream server.
    Resp(Vec<u8>),
    /// In-stream (ICY) metadata.
    Meta(Vec<u8>),
    /// Stream disconnected. 0 = closed normally, 1 = reset, 2 = timeout, 3/4 = connect failures.
    Dsco(u8),
    /// Reply to `setd`.
    Setd {
        id: u8,
        data: Vec<u8>,
    },
    Bye,
}

impl ClientMsg {
    pub fn opcode(&self) -> [u8; 4] {
        match self {
            ClientMsg::Helo(_) => *b"HELO",
            ClientMsg::Stat(_) => *b"STAT",
            ClientMsg::Resp(_) => *b"RESP",
            ClientMsg::Meta(_) => *b"META",
            ClientMsg::Dsco(_) => *b"DSCO",
            ClientMsg::Setd { .. } => *b"SETD",
            ClientMsg::Bye => *b"BYE!",
        }
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        match self {
            ClientMsg::Helo(h) => h.encode_payload(),
            ClientMsg::Stat(s) => s.encode_payload(),
            ClientMsg::Resp(b) | ClientMsg::Meta(b) => b.clone(),
            ClientMsg::Dsco(r) => vec![*r],
            ClientMsg::Setd { id, data } => {
                let mut v = vec![*id];
                v.extend_from_slice(data);
                v
            }
            ClientMsg::Bye => vec![0],
        }
    }

    /// Full frame: opcode, u32 BE payload length, payload.
    pub fn encode(&self) -> Vec<u8> {
        let payload = self.encode_payload();
        let mut v = Vec::with_capacity(8 + payload.len());
        v.extend_from_slice(&self.opcode());
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(&payload);
        v
    }
}

/// Incremental decoder for the client → server byte stream (used by the gateway and tests).
#[derive(Default)]
pub struct ClientFrameReader {
    buf: Vec<u8>,
}

impl ClientFrameReader {
    /// Largest client payload accepted. HELO capabilities, RESP headers and META are the
    /// largest legitimate frames and are well under this.
    pub const MAX_FRAME: usize = 16 * 1024;

    /// Bytes buffered while waiting for a complete frame.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_frame(&mut self) -> Result<Option<RawFrame>, ProtoError> {
        if self.buf.len() < 8 {
            return Ok(None);
        }
        let len = be32(&self.buf[4..8]) as usize;
        if len > Self::MAX_FRAME {
            return Err(ProtoError::FrameTooLarge(len));
        }
        if self.buf.len() < 8 + len {
            return Ok(None);
        }
        let opcode = self.buf[0..4].try_into().unwrap();
        let payload = self.buf[8..8 + len].to_vec();
        self.buf.drain(..8 + len);
        Ok(Some((opcode, payload)))
    }
}

// ---------------------------------------------------------------------------
// Stream request policy
// ---------------------------------------------------------------------------

/// The only stream path LMS legitimately asks a player to fetch.
pub fn stream_path_for(mac: &[u8; 6]) -> String {
    format!("/stream.mp3?player={}", format_mac(mac))
}

/// Check that a `strm` HTTP request is exactly a GET of this player's own stream, and
/// return the path. Anything else (another player, another path, an absolute URL,
/// encodings, extra parameters, a malformed request line) is refused.
pub fn validate_stream_request(request: &[u8], mac: &[u8; 6]) -> Result<String, String> {
    let line_end = request
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(request.len());
    let line =
        std::str::from_utf8(&request[..line_end]).map_err(|_| "request line is not UTF-8")?;
    let parts: Vec<&str> = line.split(' ').collect();
    if parts.len() != 3 || parts[0] != "GET" || !parts[2].starts_with("HTTP/1.") {
        return Err("malformed stream request line".into());
    }
    let want = stream_path_for(mac);
    if parts[1] != want {
        return Err(format!(
            "stream target {:?} is not this player's stream",
            parts[1]
        ));
    }
    Ok(want)
}

/// Where a player connected straight to a server (not through the gateway) may open the
/// stream a `strm` names: only on the server it is connected to (`server_ip` 0, or that
/// server's own IPv4 address), and only this player's own stream
/// ([`validate_stream_request`]), as the gateway enforces for its players. Anything else
/// (another host on the network, an internet station asked for directly) is refused, so a
/// hostile control connection can't make the host open connections wherever it likes.
pub fn direct_stream_target(
    server_ip: [u8; 4],
    port: u16,
    request: &[u8],
    mac: &[u8; 6],
    server: std::net::IpAddr,
) -> Result<std::net::SocketAddr, String> {
    let named = std::net::Ipv4Addr::from(server_ip);
    if !(named.is_unspecified() || std::net::IpAddr::V4(named) == server.to_canonical()) {
        return Err(format!("stream host {named} is not the server"));
    }
    if port == 0 {
        return Err("stream port 0".into());
    }
    validate_stream_request(request, mac)?;
    Ok(std::net::SocketAddr::new(server, port))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn need(opcode: &[u8; 4], p: &[u8], n: usize) -> Result<(), ProtoError> {
    if p.len() < n {
        Err(ProtoError::Truncated {
            opcode: *opcode,
            need: n,
            got: p.len(),
        })
    } else {
        Ok(())
    }
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Parse a MAC like `02:aa:bb:cc:dd:ee`.
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<_> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(mac)
}

pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_strm() -> Strm {
        Strm {
            command: b's',
            autostart: b'1',
            format: b'f',
            pcm_sample_size: b'?',
            pcm_sample_rate: b'?',
            pcm_channels: b'?',
            pcm_endianness: b'?',
            threshold: 255,
            spdif_enable: 0,
            transition_period: 0,
            transition_type: b'0',
            flags: 0,
            output_threshold: 0,
            slaves: 0,
            replay_gain: 0x0001_0000,
            server_port: 9000,
            server_ip: 0,
            request: b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n".to_vec(),
        }
    }

    #[test]
    fn strm_round_trips_through_server_framing() {
        let s = sample_strm();
        let frame = encode_server_frame(*b"strm", &s.encode_payload());
        let mut r = ServerFrameReader::default();
        // Feed byte by byte to exercise partial reads.
        for b in &frame[..frame.len() - 1] {
            r.push(&[*b]);
            assert!(r.next_msg().unwrap().is_none());
        }
        r.push(&frame[frame.len() - 1..]);
        assert_eq!(r.next_msg().unwrap(), Some(ServerMsg::Strm(s)));
        assert!(r.next_msg().unwrap().is_none());
    }

    #[test]
    fn two_frames_in_one_read() {
        let mut bytes = encode_server_frame(*b"vers", b"9.1.1");
        bytes.extend(encode_server_frame(*b"aude", &[1, 1]));
        let mut r = ServerFrameReader::default();
        r.push(&bytes);
        assert_eq!(r.next_msg().unwrap(), Some(ServerMsg::Vers("9.1.1".into())));
        assert_eq!(
            r.next_msg().unwrap(),
            Some(ServerMsg::Aude {
                spdif: true,
                dac: true
            })
        );
    }

    #[test]
    fn stat_is_53_bytes_and_round_trips() {
        let s = Stat {
            event: *b"STMt",
            bytes_received: 0x1_0000_0002,
            jiffies: 12345,
            elapsed_seconds: 7,
            elapsed_milliseconds: 7250,
            server_timestamp: 99,
            signal_strength: 0xffff,
            ..Default::default()
        };
        let p = s.encode_payload();
        assert_eq!(p.len(), 53);
        assert_eq!(Stat::parse(&p).unwrap(), s);
    }

    #[test]
    fn helo_round_trips_through_client_framing() {
        let h = Helo {
            device_id: DEVICE_ID_SOFTWARE,
            revision: 0,
            mac: [2, 0, 0, 0, 0, 1],
            uuid: [0; 16],
            wlan_channels: 0,
            bytes_received: 0,
            lang: *b"en",
            capabilities: "Model=lyrionanywhere,flc,mp3".into(),
        };
        let mut r = ClientFrameReader::default();
        r.push(&ClientMsg::Helo(h.clone()).encode());
        let (op, p) = r.next_frame().unwrap().unwrap();
        assert_eq!(&op, b"HELO");
        assert_eq!(Helo::parse(&p).unwrap(), h);
    }

    #[test]
    fn truncated_strm_is_an_error_not_a_panic() {
        let frame = encode_server_frame(*b"strm", b"s1");
        let mut r = ServerFrameReader::default();
        r.push(&frame);
        assert!(matches!(r.next_msg(), Err(ProtoError::Truncated { .. })));
    }

    #[test]
    fn audg_fixed_volume_when_adjust_off() {
        let a = Audg {
            old_left: 0,
            old_right: 0,
            adjust: 0,
            preamp: 255,
            new_left: 100,
            new_right: 100,
        };
        assert_eq!(a.gains(), (1.0, 1.0));
        let a = Audg {
            adjust: 1,
            new_left: 0x8000,
            new_right: 0x10000,
            ..a
        };
        assert_eq!(a.gains(), (0.5, 1.0));
    }

    #[test]
    fn stream_request_policy() {
        let mac = [2, 0, 0, 0, 0, 1];
        let good = b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\nIcy-MetaData: 1\r\n\r\n";
        assert_eq!(
            validate_stream_request(good, &mac).unwrap(),
            "/stream.mp3?player=02:00:00:00:00:01"
        );
        let attacks: &[&[u8]] = &[
            b"GET /stream.mp3?player=aa:bb:cc:dd:ee:ff HTTP/1.0\r\n\r\n",
            b"GET /music/123/download HTTP/1.0\r\n\r\n",
            b"GET /stream.mp3/../jsonrpc.js?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n",
            b"GET /stream.mp3?player=02:00:00:00:00:01&x=1 HTTP/1.0\r\n\r\n",
            b"POST /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n",
            b"GET http://evil.example/stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n",
            b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0 X\r\n\r\n",
            b"",
            b"GET /stream.mp3?player=02:00:00:00:00:01\r\n\r\n",
            b"GET /%2e%2e/stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n",
            b"GET /stream.mp3?player=02%3A00%3A00%3A00%3A00%3A01 HTTP/1.0\r\n\r\n",
            b"GET  /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n",
            b"\xff\xfe /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n",
        ];
        for a in attacks {
            assert!(
                validate_stream_request(a, &mac).is_err(),
                "accepted {:?}",
                String::from_utf8_lossy(a)
            );
        }
    }

    /// A directly connected host opens a stream only on its own server, and only its own
    /// stream (review low: the CLI connected wherever the control connection said).
    #[test]
    fn direct_streams_go_only_to_the_server() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
        let mac = [2, 0, 0, 0, 0, 1];
        let good = b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n";
        let server = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 10));
        let at = |ip: IpAddr| Ok(SocketAddr::new(ip, 9000));
        assert_eq!(
            direct_stream_target([0; 4], 9000, good, &mac, server),
            at(server)
        );
        assert_eq!(
            direct_stream_target([10, 0, 0, 10], 9000, good, &mac, server),
            at(server)
        );
        // An IPv4-mapped server address is the same server.
        let mapped = IpAddr::V6(Ipv4Addr::new(10, 0, 0, 10).to_ipv6_mapped());
        assert!(direct_stream_target([10, 0, 0, 10], 9000, good, &mac, mapped).is_ok());
        let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert_eq!(direct_stream_target([0; 4], 9000, good, &mac, v6), at(v6));
        for other in [
            [10, 0, 0, 11],
            [127, 0, 0, 1],
            [192, 0, 2, 7],
            [255, 255, 255, 255],
            [169, 254, 169, 254],
        ] {
            assert!(
                direct_stream_target(other, 9000, good, &mac, server).is_err(),
                "{other:?}"
            );
        }
        assert!(direct_stream_target([127, 0, 0, 1], 9000, good, &mac, v6).is_err());
        // The server, but not this player's stream, or no port.
        let other_player = b"GET /stream.mp3?player=aa:bb:cc:dd:ee:ff HTTP/1.0\r\n\r\n";
        assert!(direct_stream_target([0; 4], 9000, other_player, &mac, server).is_err());
        assert!(direct_stream_target([0; 4], 9000, b"GET /x HTTP/1.0", &mac, server).is_err());
        assert!(direct_stream_target([0; 4], 0, good, &mac, server).is_err());
    }

    #[test]
    fn mac_parse_and_format() {
        let m = parse_mac("02:ab:00:10:ff:01").unwrap();
        assert_eq!(format_mac(&m), "02:ab:00:10:ff:01");
        assert!(parse_mac("02:ab").is_none());
    }
}
