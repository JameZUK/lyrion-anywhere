// Player worker: runs the WASM core, owns the gateway WebSocket and stream fetches, and
// feeds PCM to the AudioWorklet through a MessagePort. Keeping this off the main thread
// means UI work never stalls decoding.
//
// Reconnects automatically: a dropped socket resumes the same gateway session (LMS never
// sees the player leave), and a broken stream download resumes from its byte offset.

import init, { WasmPlayer, WasmSegmenter } from './pkg/anywhere_wasm.js';
import { CLOSE, Outbox, backoffMs, closeAction, streamFailureAction } from './reconnect.js';

const TICK_MS = 20;
// PCM queued in the worklet ahead of playback. Small, so pause/stop feel immediate.
const WORKLET_AHEAD_SECS = 0.3;
const CODECS = 'flc,mp3,ogg';
// Give up resuming slightly before the gateway's default 60 s grace runs out.
const RESUME_FOR_MS = 55_000;
const STREAM_RETRIES = 12;
// Acknowledge stream bytes this often; the gateway stays within its replay window of the
// last ack, so a broken download can always resume.
const ACK_EVERY = 256 * 1024;

let player = null;
let wasmMemory = null; // the core's linear memory (it only ever grows)
let ws = null;
let token = '';
let session = '';
let secret = '';
let sink = null; // MessagePort to the AudioWorklet
let readyRate = 0; // sample rate the main thread's AudioContext is running at
let pulled = 0;
let played = 0;
let lastOutput = '';
let stream = null; // { id, n, controller }
let streamId = 0;
let tickTimer = null;
let open = false;
let attempt = 0;
let disconnectedAt = 0;
// Frames received in this session; on resume the gateway resends anything after this.
let received = 0;
const outbox = new Outbox(2000);

// Output mode: 'worklet' (PCM to an AudioWorklet) or 'media' (fMP4/FLAC segments for an
// <audio> element on the main thread, which keeps playing in the background on iOS).
let outputMode = 'worklet';
// Seconds of audio to keep queued in the media element: enough to ride out timer throttling
// while the page is in the background, small enough that buffers stay modest.
const MEDIA_AHEAD_SECS = 8;
let segmenter = null;
let mediaWaiting = false; // flushed; waiting for the main thread's new timeline position

const now = () => performance.now();
const log = (msg) => postMessage({ log: msg });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function attachSink(port) {
  sink = port;
  pulled = played = 0;
  sink.onmessage = (m) => {
    if (m.data.played && player) {
      played += m.data.played;
      player.played(m.data.played);
    }
  };
}

onmessage = async (e) => {
  const msg = e.data;
  if (msg.cmd === 'start') {
    token = msg.token;
    outputMode = msg.mode === 'media' ? 'media' : 'worklet';
    if (msg.port) attachSink(msg.port);
    const wasm = await init();
    wasmMemory = wasm.memory ?? null;
    connect(false);
  } else if (msg.cmd === 'start-sink') {
    attachSink(msg.port);
  } else if (msg.cmd === 'rate-ready') {
    readyRate = msg.rate;
  } else if (msg.cmd === 'played') {
    // Media mode: frames the <audio> element has actually played.
    if (player && msg.frames > 0) {
      played += msg.frames;
      player.played(msg.frames);
    }
  } else if (msg.cmd === 'media-drained') {
    // Media mode: the element played everything appended (it is waiting for more). Count
    // it all, so the core sees the end of the playlist and LMS stops ("played out").
    if (player && segmenter) {
      if (segmenter.pending_frames() > 0) {
        // Something is still held back (a partial frame): send it; the element plays on.
        postSegment(segmenter.flush());
      } else {
        // Everything pulled has been played. The core caps this at what was pulled, so it
        // can't over-count; the worker's own estimate catches up too.
        player.played(Number.MAX_SAFE_INTEGER);
        played = pulled;
      }
    }
  } else if (msg.cmd === 'media-time') {
    // Media mode, after a flush: continue the timeline where the element now is.
    if (segmenter) {
      segmenter.set_time(msg.at);
      postMessage({ media: { span: { at: segmenter.emitted_seconds(), rate: segmenter.rate() } } });
    }
    mediaWaiting = false;
  } else if (msg.cmd === 'token') {
    // A renewed access token from the page; reconnect if we were waiting for it.
    token = msg.token;
    if (msg.reconnect && !open) connect(session !== '');
  }
};

function connect(resume) {
  const url = new URL('/v1/player', self.location.origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => {
    ws.send(resume ? JSON.stringify({ token, resume: { session, secret, received } }) : token);
  };
  ws.onmessage = (m) => (typeof m.data === 'string' ? onGreeting(JSON.parse(m.data)) : onFrames(m.data));
  ws.onclose = (e) => onClose(e.code, e.reason);
}

function onGreeting(g) {
  session = g.session;
  secret = g.secret;
  open = true;
  attempt = 0;
  if (!g.resumed || !player) {
    // A fresh session: LMS sees a new connection, so start from a clean player.
    closeStream();
    outbox.clear();
    received = 0;
    player = new WasmPlayer(g.mac, g.name, CODECS);
    player.connected(now());
    log(`connected as ${g.name} (${g.mac})`);
  } else {
    const dropped = outbox.dropped;
    log(`resumed after ${Math.round((now() - disconnectedAt) / 100) / 10} s${dropped ? `, ${dropped} queued messages dropped` : ''}`);
  }
  postMessage({ connection: 'connected', player: { mac: g.mac, name: g.name } });
  for (const bytes of outbox.drain()) ws.send(bytes);
  const os = player.open_stream();
  if (os) sendAck(os[0], os[1]);
  runActions();
  if (!tickTimer) tickTimer = setInterval(tick, TICK_MS);
}

function onFrames(data) {
  received++;
  try {
    player.on_control(new Uint8Array(data), now());
  } catch (err) {
    log(`protocol error: ${err}`);
    ws.close();
  }
  runActions();
}

function onClose(code, reason) {
  const wasOpen = open;
  open = false;
  if (wasOpen) disconnectedAt = now();
  const action = closeAction(code);
  log(`gateway connection closed (${code}${reason ? `: ${reason}` : ''})`);
  if (code === CLOSE.POLICY && reason === 'unauthorised') {
    // Most likely an expired access token: the page renews it and sends it back.
    postMessage({ connection: 'reauthenticating', authFailed: true });
    return;
  }
  if (action === 'stop') {
    postMessage({ connection: 'closed' });
    return;
  }
  let resume = action === 'resume' && session !== '' && now() - disconnectedAt < RESUME_FOR_MS;
  if (!resume) session = '';
  const delay = backoffMs(attempt++);
  postMessage({ connection: resume ? 'reconnecting' : 'restarting' });
  setTimeout(() => connect(resume), delay);
}

function send(bytes) {
  if (open && ws.readyState === WebSocket.OPEN) ws.send(bytes);
  else outbox.push(bytes);
}

// Acks are only useful live; one lost while disconnected is replaced by the next.
function sendAck(n, bytes) {
  if (open && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ ack: { stream: n, bytes } }));
  }
}

function runActions() {
  for (const a of player.take_actions()) {
    if (a.type === 'send') send(a.bytes);
    else if (a.type === 'open') openStream(a.stream);
    else if (a.type === 'close') closeStream();
  }
}

function closeStream() {
  if (stream) {
    if (stream.controller) stream.controller.abort();
    stream = null;
  }
}

function openStream(n) {
  closeStream();
  const id = ++streamId;
  stream = { id, n, controller: null };
  fetchStream(id, n);
}

async function fetchStream(id, n) {
  const current = () => stream && stream.id === id;
  let headersSeen = false;
  let lastAck = 0;
  for (let tries = 0; ; tries++) {
    // Resume from what the core has already received for this stream.
    const os = player.open_stream();
    const offset = os && os[0] === n ? os[1] : 0;
    const controller = new AbortController();
    stream.controller = controller;
    let status = 0;
    try {
      const q = offset ? `?offset=${offset}` : '';
      const resp = await fetch(`/v1/stream/${session}/${n}${q}`, {
        headers: { Authorization: `Bearer ${token}` },
        signal: controller.signal,
        cache: 'no-store',
      });
      if (!current()) return;
      status = resp.status;
      if (!resp.ok) throw new Error(`stream HTTP ${resp.status}`);
      // From here on, a failure is the connection breaking mid-download: retryable.
      status = 0;
      if (!headersSeen) {
        headersSeen = true;
        const ct = resp.headers.get('content-type') || 'application/octet-stream';
        player.on_stream_headers(new TextEncoder().encode(`HTTP/1.0 200 OK\r\nContent-Type: ${ct}\r\n\r\n`), now());
      } else {
        log(`stream ${n} resumed at byte ${offset}`);
      }
      const reader = resp.body.getReader();
      for (;;) {
        // Back-pressure: stop reading while the core's input buffer is full.
        while (current() && !player.wants_stream_data()) await sleep(50);
        if (!current()) return;
        const { done, value } = await reader.read();
        if (!current()) return;
        if (done) {
          player.on_stream_end(now());
          runActions();
          return;
        }
        tries = 0;
        player.on_stream_data(value, now());
        const got = player.open_stream();
        if (got && got[0] === n && got[1] - lastAck >= ACK_EVERY) {
          lastAck = got[1];
          sendAck(n, lastAck);
        }
      }
    } catch (err) {
      if (err.name === 'AbortError' || !current()) return;
      if (streamFailureAction(status) === 'fail' || tries >= STREAM_RETRIES) {
        log(`stream ${n} failed after ${tries} retries at byte ${offset}: ${err.message} (HTTP ${status || 'none'})`);
        player.on_stream_error(now());
        runActions();
        return;
      }
      const at = player.open_stream();
      log(`stream ${n} interrupted at byte ${at && at[0] === n ? at[1] : offset} (${err.message}); retrying`);
      await sleep(backoffMs(tries));
      if (!current()) return;
    }
  }
}

function tick() {
  if (!player) return;
  player.tick(now());
  runActions();

  const status = JSON.parse(player.status_json());
  if (outputMode === 'media') {
    mediaTick(status.output);
  } else {
    workletTick(status.output);
  }
  lastOutput = status.output;
  status.connection = open ? 'connected' : 'reconnecting';
  // Memory diagnostics: iOS reloads a page whose process uses too much memory.
  status.wasm_bytes = wasmMemory ? wasmMemory.buffer.byteLength : 0;
  status.mode = outputMode;
  postMessage({ status });
}

function workletTick(output) {
  // After stop/flush, drop audio already queued in the worklet.
  if (output === 'Stopped' && lastOutput !== 'Stopped') {
    sink.postMessage({ flush: true });
    pulled = played = 0;
  }

  const rate = player.output_rate();
  if (rate && rate !== readyRate) {
    postMessage({ rate }); // main thread (re)creates the AudioContext at this rate
  } else if (rate) {
    const target = rate * WORKLET_AHEAD_SECS;
    while (pulled - played < target) {
      const buf = new Float32Array(2048 * 2);
      const frames = player.pull(buf);
      if (!frames) break;
      const chunk = buf.slice(0, frames * 2);
      sink.postMessage({ pcm: chunk }, [chunk.buffer]);
      pulled += frames;
    }
  }
}

function postSegment(bytes) {
  if (bytes) postMessage({ media: { segment: bytes } }, [bytes.buffer]);
}

function mediaTick(output) {
  // Stop/flush: the main thread empties the element's buffer and tells us where to resume.
  if (output === 'Stopped' && lastOutput !== 'Stopped') {
    pulled = played = 0;
    if (segmenter) {
      mediaWaiting = true;
      postMessage({ media: { flush: true } });
    }
  }
  // The core reports e.g. "Paused { until: None }": compare by prefix.
  const paused = output.startsWith('Paused');
  const wasPaused = String(lastOutput).startsWith('Paused');
  if (paused && !wasPaused) postMessage({ media: { pause: true } });
  if (output === 'Playing' && lastOutput !== 'Playing') postMessage({ media: { play: true } });
  if (mediaWaiting) return;

  const rate = player.output_rate();
  if (!rate) return;
  if (!segmenter || segmenter.rate() !== rate) {
    if (segmenter) {
      postSegment(segmenter.flush()); // the rest of the old rate first
      segmenter.set_rate(rate);
    } else {
      segmenter = new WasmSegmenter(rate);
    }
    postMessage({ media: { init: segmenter.init_segment(), span: { at: segmenter.emitted_seconds(), rate } } });
  }
  let exhausted = false;
  while (pulled - played < rate * MEDIA_AHEAD_SECS) {
    const buf = new Float32Array(4096 * 2);
    const frames = player.pull(buf);
    if (!frames) {
      exhausted = player.audio_exhausted();
      break;
    }
    segmenter.push(buf.subarray(0, frames * 2));
    pulled += frames;
  }
  postSegment(segmenter.segment());
  // The core has nothing more to give (end of the playlist, or waiting for the network):
  // send the partial frame too, so the element can play every last frame. Only then: a
  // flush mid-stream would pad the frame with silence.
  if (exhausted && segmenter.pending_frames() > 0) {
    const n = segmenter.pending_frames();
    postSegment(segmenter.flush());
    log(`media: sent the last ${n} frames`);
  }
}
