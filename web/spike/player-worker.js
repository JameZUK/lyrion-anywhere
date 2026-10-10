// Player worker: runs the WASM core, owns the gateway WebSocket and stream fetches, and
// feeds PCM to the AudioWorklet through a MessagePort. Keeping this off the main thread
// means UI work never stalls decoding.

import init, { WasmPlayer } from './pkg/anywhere_wasm.js';

const TICK_MS = 20;
// PCM queued in the worklet ahead of playback. Small, so pause/stop feel immediate.
const WORKLET_AHEAD_SECS = 0.3;

let player = null;
let ws = null;
let token = '';
let session = '';
let sink = null; // MessagePort to the AudioWorklet
let readyRate = 0; // sample rate the main thread's AudioContext is running at
let pulled = 0;
let played = 0;
let lastOutput = '';
let stream = null; // { id, controller }
let streamId = 0;

const now = () => performance.now();
const log = (msg) => postMessage({ log: msg });

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
    connect();
  } else if (msg.cmd === 'start-sink') {
    attachSink(msg.port);
  } else if (msg.cmd === 'rate-ready') {
    readyRate = msg.rate;
  }
};

function connect() {
  const url = new URL('/v1/player', self.location.origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => ws.send(token);
  ws.onmessage = (m) => {
    if (typeof m.data === 'string') {
      const hello = JSON.parse(m.data);
      session = hello.session;
      player = new WasmPlayer(hello.mac, hello.name, 'flc,mp3,ogg');
      player.connected(now());
      log(`connected as ${hello.name} (${hello.mac})`);
      runActions();
      setInterval(tick, TICK_MS);
      return;
    }
    try {
      player.on_control(new Uint8Array(m.data), now());
    } catch (err) {
      log(`protocol error: ${err}`);
      ws.close();
    }
    runActions();
  };
  ws.onclose = (e) => {
    log(`gateway connection closed (${e.code}${e.reason ? `: ${e.reason}` : ''})`);
    postMessage({ closed: true });
  };
}

function runActions() {
  for (const a of player.take_actions()) {
    if (a.type === 'send') {
      if (ws.readyState === WebSocket.OPEN) ws.send(a.bytes);
    } else if (a.type === 'open') {
      openStream(a.stream);
    } else if (a.type === 'close') {
      closeStream();
    }
  }
}

function closeStream() {
  if (stream) {
    stream.controller.abort();
    stream = null;
  }
}

async function openStream(n) {
  closeStream();
  const id = ++streamId;
  const controller = new AbortController();
  stream = { id, controller };
  const current = () => stream && stream.id === id;
  try {
    const resp = await fetch(`/v1/stream/${session}/${n}`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: controller.signal,
      cache: 'no-store',
    });
    if (!current()) return;
    if (!resp.ok) throw new Error(`stream HTTP ${resp.status}`);
    const ct = resp.headers.get('content-type') || 'application/octet-stream';
    player.on_stream_headers(new TextEncoder().encode(`HTTP/1.0 200 OK\r\nContent-Type: ${ct}\r\n\r\n`), now());
    const reader = resp.body.getReader();
    for (;;) {
      // Back-pressure: stop reading while the core's input buffer is full.
      while (current() && !player.wants_stream_data()) {
        await new Promise((r) => setTimeout(r, 50));
      }
      if (!current()) return;
      const { done, value } = await reader.read();
      if (!current()) return;
      if (done) {
        player.on_stream_end(now());
        break;
      }
      player.on_stream_data(value, now());
    }
  } catch (err) {
    if (err.name !== 'AbortError' && current()) {
      log(`stream error: ${err.message}`);
      player.on_stream_error(now());
    }
  }
  runActions();
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
  postMessage({ status });
}
