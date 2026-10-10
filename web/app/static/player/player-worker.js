// Player worker: runs the WASM core, owns the gateway WebSocket and stream fetches, and
// feeds PCM to the AudioWorklet through a MessagePort. Keeping this off the main thread
// means UI work never stalls decoding.
//
// Reconnects automatically: a dropped socket resumes the same gateway session (LMS never
// sees the player leave), and a broken stream download resumes from its byte offset.

import init, { WasmPlayer } from './pkg/anywhere_wasm.js';
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
    attachSink(msg.port);
    await init();
    connect(false);
  } else if (msg.cmd === 'start-sink') {
    attachSink(msg.port);
  } else if (msg.cmd === 'rate-ready') {
    readyRate = msg.rate;
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
  // After stop/flush, drop audio already queued in the worklet.
  if (status.output === 'Stopped' && lastOutput !== 'Stopped') {
    sink.postMessage({ flush: true });
    pulled = played = 0;
  }
  lastOutput = status.output;

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
  status.connection = open ? 'connected' : 'reconnecting';
  postMessage({ status });
}
