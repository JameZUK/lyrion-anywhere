// Player worker: runs the WASM core, owns the gateway WebSocket and stream fetches, and
// feeds PCM to the AudioWorklet through a MessagePort. Keeping this off the main thread
// means UI work never stalls decoding.
//
// Reconnects automatically: a dropped socket resumes the same gateway session (LMS never
// sees the player leave), and a broken stream download resumes from its byte offset.

// Helpers load with the build version (`?v=`, from this worker's own URL), so a browser
// can't pair an old cached copy with a new app.
const V = new URL(self.location.href).searchParams.get('v');
// Messages that arrive while the helpers load wait here, then go to the real handler.
const early = [];
onmessage = (e) => early.push(e);
const VQ = V && /^[\w.-]{1,64}$/.test(V) ? `?v=${V}` : '';
const { AuthWait, CLOSE, FlushWatch, Outbox, RateRequest, SeekTimer, backoffMs, closeAction, describeSeek, loadCore, qualityMessage, sanitiseQuality, signInMessage, socketSilent, streamFailureAction } =
  await import(`./reconnect.js${VQ}`);
const { Presentation } = await import(`./presentation.js${VQ}`);

const TICK_MS = 20;
// PCM queued in the worklet ahead of playback. Small, so pause/stop feel immediate.
const WORKLET_AHEAD_SECS = 0.3;
const CODECS = 'flc,mp3,ogg';
// After a seek (or from stopped), start once this much audio is decoded if the stream is
// arriving quickly, rather than waiting for all of LMS's buffer threshold. The output's
// queue then fills while playing.
const QUICK_START_MS = 500;
// Give up resuming slightly before the gateway's default 60 s grace runs out.
const RESUME_FOR_MS = 55_000;
const STREAM_RETRIES = 12;
// Acknowledge stream bytes this often; the gateway stays within its replay window of the
// last ack, so a broken download can always resume.
const ACK_EVERY = 256 * 1024;

let player = null;
let core = null; // the WebAssembly core's module (classes, and its memory)
let wasmMemory = null; // the core's linear memory (it only ever grows)
let ws = null;
let token = '';
// The stream level wanted ('original', 'high' or 'low'), and whether the gateway lets a
// session change it (null until known; docs/plans/streaming-quality.md).
let quality = 'original';
let qualityFeature = null;
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
// When the socket last brought anything (LMS polls every 5 s): a silent one has died.
let lastHeard = 0;
// Frames received in this session; on resume the gateway resends anything after this.
let received = 0;
const outbox = new Outbox(2000);
const rateRequest = new RateRequest();
const authWait = new AuthWait();
// Seek timing: from LMS's flush to the new audio being heard (logged once heard).
const seekTimer = new SeekTimer();
const flushWatch = new FlushWatch();
let seekFlushAt = 0;
// Bumped at every flush: the worklet tags its "played" reports with it, so frames of the
// old position reported after a flush are not counted against the new one.
let epoch = 0;
// LMS's volume last reported to the page ([left, right] linear gains).
let lastVolume = null;
// Web Audio mode: what the worklet has rendered, and (from the page's output timestamps)
// how much of it is audible. That is what the core reports to LMS, for sync.
const presentation = new Presentation();
// A timed start is armed in the worklet, which starts itself at the exact frame.
let timedStart = false;
// The hold last sent to the worklet.
let workletHeld = false;

// Output mode: 'worklet' (PCM to an AudioWorklet) or 'media' (fMP4/FLAC segments for an
// <audio> element on the main thread, which keeps playing in the background on iOS).
let outputMode = 'worklet';
// Seconds of audio to keep queued in the media element: enough to ride out timer throttling
// while the page is in the background, small enough that buffers stay modest.
const MEDIA_AHEAD_SECS = 8;
let segmenter = null;
let mediaVariant = 0; // init-segment variant the browser's self-test accepted
let mediaWaiting = false; // flushed; waiting for the main thread's new timeline position

const now = () => performance.now();
// Absolute time (ms), comparable with the page's: each has its own performance.now().
const absNow = () => performance.timeOrigin + performance.now();
const log = (msg) => postMessage({ log: msg });

function seekMark(stage) {
  const done = seekTimer.mark(stage, now());
  if (!done) return;
  log(describeSeek(done));
  postMessage({ seekTiming: { ...done, flushAt: performance.timeOrigin + seekFlushAt } });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function attachSink(port) {
  sink = port;
  pulled = played = 0;
  // A new AudioContext: its frames count from 0 again, and its clock comes with it.
  presentation.reset();
  timedStart = false;
  workletHeld = false;
  sink.postMessage({ flush: true, epoch });
  sink.onmessage = (m) => {
    const d = m.data;
    if (!d || !player || d.epoch !== epoch) return;
    // Rendered (not yet necessarily heard): paces how far ahead the worker fills.
    if (Number.isInteger(d.played) && d.played > 0) played += d.played;
    presentation.add(d.runs);
  };
}

/**
 * LMS flushed the player (a seek, a stop, or a new session): drop the audio queued in the
 * output at once. Driven by the core's flush count, not by watching the output state on a
 * timer: LMS sends the flush and the new stream back to back, and a tick can miss the
 * stopped state in between (then the old queue, seconds of it in media mode, played on).
 */
function flushOutput() {
  pulled = played = 0;
  epoch++;
  presentation.dropRuns();
  timedStart = false;
  if (outputMode === 'media') {
    if (segmenter) {
      mediaWaiting = true;
      postMessage({ media: { flush: true } });
    }
  } else if (sink) {
    sink.postMessage({ flush: true, epoch });
  }
}

/** Tell the page when LMS's volume changes; the page applies it at the output. */
function reportVolume() {
  const v = player.volume();
  const l = Number(v[0]);
  const r = Number(v[1]);
  if (lastVolume && lastVolume[0] === l && lastVolume[1] === r) return;
  lastVolume = [l, r];
  postMessage({ volume: lastVolume });
}

onmessage = async (e) => {
  const msg = e.data;
  if (msg.cmd === 'start') {
    token = msg.token;
    quality = sanitiseQuality(msg.quality);
    outputMode = msg.mode === 'media' ? 'media' : 'worklet';
    mediaVariant = Number.isInteger(msg.variant) ? msg.variant : 0;
    if (msg.port) attachSink(msg.port);
    // The build version in the URLs: a browser can't pair an old cached copy with a new app.
    core = await loadCore(self.location.href, (u) => import(u));
    wasmMemory = core.wasm.memory ?? null;
    connect(false);
  } else if (msg.cmd === 'start-sink') {
    attachSink(msg.port);
  } else if (msg.cmd === 'rate-ready') {
    readyRate = msg.rate;
  } else if (msg.cmd === 'clock') {
    // Web Audio mode: which context time is heard when (from getOutputTimestamp).
    presentation.setClock(msg.ctx, msg.at, msg.rate);
  } else if (msg.cmd === 'played') {
    // Media mode: frames the <audio> element has actually played, and when the position
    // was read (absolute ms). Reports sent while a flush is under way are of the old
    // position. A report of 0 frames still says where it is, and when.
    const frames = Number.isInteger(msg.frames) && msg.frames >= 0 ? msg.frames : 0;
    if (player && !mediaWaiting) {
      if (frames > 0) seekMark('audible');
      played += frames;
      const at = Number.isFinite(msg.at) ? Math.min(now(), Math.max(0, msg.at - performance.timeOrigin)) : now();
      player.presented(frames, at);
    }
  } else if (msg.cmd === 'media-drained') {
    // Media mode: the element played everything appended (it is waiting for more). Count
    // it all, so the core sees the end of the playlist and LMS stops ("played out").
    // Only while playing: a drain reported around a pause or a flush is not the end.
    if (player && segmenter && !mediaWaiting && lastOutput === 'Playing') {
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
  } else if (msg.cmd === 'quality') {
    // From the next stream on (the next track, or a seek).
    const q = sanitiseQuality(msg.quality);
    if (q !== quality) {
      quality = q;
      sendQuality();
    }
  } else if (msg.cmd === 'token') {
    // A renewed access token from the page; reconnect if we were waiting for it, whether
    // it came with the page's reply to authFailed or later (a renewal retried after being
    // offline arrives as a plain token update).
    token = msg.token;
    if (authWait.token(open)) connect(session !== '');
  }
};

/** Tell the gateway the level now, if it can take it mid-session; otherwise it is asked for at the next sign-in. */
function sendQuality() {
  if (open && qualityFeature === true && ws?.readyState === WebSocket.OPEN) ws.send(qualityMessage(quality));
}

/** Ask the gateway once whether a session may change its level. */
async function learnQualityFeature() {
  if (qualityFeature !== null) return;
  try {
    const r = await fetch(new URL('/v1/me', self.location.origin), {
      headers: { Authorization: `Bearer ${token}`, 'X-Anywhere-Client': '1' },
      cache: 'no-store',
    });
    if (!r.ok) return;
    const me = await r.json();
    qualityFeature = me?.features?.stream_quality === true;
    sendQuality();
  } catch {
    // Offline or refused: asked again on the next connection.
  }
}

function connect(resume) {
  const url = new URL('/v1/player', self.location.origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => {
    ws.send(signInMessage(token, resume ? { session, secret, received } : null, quality));
  };
  ws.onmessage = (m) => {
    lastHeard = now();
    if (typeof m.data === 'string') onGreeting(JSON.parse(m.data));
    else onFrames(m.data);
  };
  ws.onclose = (e) => onClose(e.code, e.reason);
}

/**
 * The socket went silent without closing (it died on a network change, and the browser
 * hasn't noticed): stop waiting for it and resume on a new one, while the gateway still
 * holds the session.
 */
function abandonSocket() {
  const dead = ws;
  dead.onopen = dead.onmessage = dead.onclose = null;
  try {
    dead.close();
  } catch {
    /* already closing */
  }
  onClose(CLOSE.ABNORMAL, 'no messages from the server');
}

function onGreeting(g) {
  session = g.session;
  secret = g.secret;
  open = true;
  lastHeard = now();
  void learnQualityFeature();
  attempt = 0;
  if (!g.resumed || !player) {
    // A fresh session: LMS sees a new connection, so start from a clean player.
    closeStream();
    outbox.clear();
    received = 0;
    player = new core.WasmPlayer(g.mac, g.name, CODECS);
    // Volume is applied at the output, where a change is heard at once.
    player.set_host_volume(true);
    // So are LMS's sync corrections: a timed start, pauses and skips.
    player.set_output_corrections(true);
    player.set_quick_start(QUICK_START_MS);
    flushWatch.reset();
    lastVolume = null;
    seekTimer.cancel();
    flushOutput(); // anything still queued belongs to the old session
    reportVolume();
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
    if (flushWatch.changed(player.flush_count())) {
      flushOutput();
      seekFlushAt = now();
      seekTimer.start(seekFlushAt);
    }
    reportVolume();
  } catch (err) {
    log(`protocol error: ${err}`);
    ws.close();
  }
  runActions();
  applyCorrections();
}

/**
 * Apply LMS's sync corrections at the output, where they take effect at once and exactly
 * rather than after the audio queued ahead: in the worklet (to the frame), or on the media
 * element (as near as it allows).
 */
function applyCorrections() {
  if (!player) return;
  for (const c of player.take_corrections()) {
    if (outputMode === 'media') {
      if (c.type === 'start') postMessage({ media: { startAt: performance.timeOrigin + c.at } });
      else if (c.type === 'pause') postMessage({ media: { pauseFor: c.ms } });
      else if (c.type === 'skip') postMessage({ media: { skip: c.ms } });
      continue;
    }
    if (!sink || !readyRate) continue;
    const frames = (ms) => Math.round((ms * readyRate) / 1000);
    if (c.type === 'pause') sink.postMessage({ silence: frames(c.ms) });
    else if (c.type === 'skip') sink.postMessage({ skip: frames(c.ms) });
    else if (c.type === 'start') {
      // The context frame heard at the start time; without the page's clock, the core's
      // own timing (a tick) starts it instead.
      const f = presentation.frameFor(performance.timeOrigin + c.at, absNow());
      if (f !== null) {
        sink.postMessage({ startAt: f });
        timedStart = true;
        workletHeld = true;
      }
    }
  }
}

function onClose(code, reason) {
  const wasOpen = open;
  open = false;
  if (wasOpen) disconnectedAt = now();
  const action = closeAction(code);
  log(`gateway connection closed (${code}${reason ? `: ${reason}` : ''})`);
  if (code === CLOSE.POLICY && reason === 'unauthorised') {
    // Most likely an expired access token: the page renews it and sends it back.
    authWait.refused();
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
    else if (a.type === 'open') {
      seekMark('stream');
      openStream(a.stream);
    }
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
  let renewed = false;
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
        seekMark('response');
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
        renewed = false;
        seekMark('bytes');
        player.on_stream_data(value, now());
        const got = player.open_stream();
        if (got && got[0] === n && got[1] - lastAck >= ACK_EVERY) {
          lastAck = got[1];
          sendAck(n, lastAck);
        }
      }
    } catch (err) {
      if (err.name === 'AbortError' || !current()) return;
      const action = streamFailureAction(status, renewed);
      if (action === 'renew') {
        // Most likely the access token expired (a long pause, or a page in the background
        // whose renewal timer was held back): get a new one, then carry on.
        renewed = true;
        log(`stream ${n}: access token refused; renewing`);
        await renewToken();
        if (!current()) return;
        continue;
      }
      if (action === 'fail' || tries >= STREAM_RETRIES) {
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

/** Ask the page for a new access token; resolves once it arrives (or after 15 s). */
async function renewToken() {
  const old = token;
  postMessage({ authFailed: true });
  for (let waited = 0; token === old && waited < 15_000; waited += 100) await sleep(100);
}

function tick() {
  if (!player) return;
  if (open && socketSilent(lastHeard, now())) {
    log('no messages from the server for a while; reconnecting');
    abandonSocket();
  }
  if (outputMode !== 'media') {
    // What has become audible, exact as of now.
    const heard = presentation.take(absNow());
    if (heard > 0) seekMark('audible');
    player.presented(heard, now());
  }
  player.tick(now());
  runActions();
  applyCorrections();

  const status = JSON.parse(player.status_json());
  if (status.output === 'Playing') seekMark('started');
  if (outputMode === 'media') {
    mediaTick(status.output);
  } else {
    holdWorklet(status.output);
    workletTick();
  }
  lastOutput = status.output;
  status.connection = open ? 'connected' : 'reconnecting';
  // Memory diagnostics: iOS reloads a page whose process uses too much memory.
  status.wasm_bytes = wasmMemory ? wasmMemory.buffer.byteLength : 0;
  status.mode = outputMode;
  postMessage({ status });
}

/**
 * Silence the worklet (keeping its queue) unless playing, as LMS's pauses take effect at
 * once. A timed start armed in the worklet releases the hold itself, at the exact frame:
 * leave it alone until the core's start time has come.
 */
function holdWorklet(output) {
  if (!sink) return;
  const timed = timedStart && player.start_due() !== undefined;
  if (timedStart && !timed) timedStart = false;
  if (timed) return;
  const hold = output !== 'Playing';
  if (hold !== workletHeld) {
    workletHeld = hold;
    sink.postMessage({ hold });
  }
}

function workletTick() {
  const rate = player.output_rate();
  if (rate && rate !== readyRate) {
    // The main thread (re)creates the AudioContext at this rate: ask once, then wait.
    if (rateRequest.want(rate, readyRate)) postMessage({ rate });
  } else if (rate) {
    const target = rate * WORKLET_AHEAD_SECS;
    while (pulled - played < target) {
      const buf = new Float32Array(2048 * 2);
      const frames = player.pull(buf);
      if (!frames) break;
      const chunk = buf.slice(0, frames * 2);
      sink.postMessage({ pcm: chunk }, [chunk.buffer]);
      seekMark('queued');
      pulled += frames;
    }
  }
}

function postSegment(bytes) {
  if (!bytes) return;
  postMessage({ media: { segment: bytes } }, [bytes.buffer]);
  seekMark('queued');
}

function mediaTick(output) {
  // (A flush is handled as it arrives: see flushOutput.)
  // The core reports e.g. "Paused { until: None }": compare by prefix.
  // Loaded counts as paused: audio is queued ahead of a synced start, which must not play
  // until the start.
  const paused = output.startsWith('Paused') || output === 'Loaded';
  const wasPaused = String(lastOutput).startsWith('Paused') || lastOutput === 'Loaded';
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
      segmenter = new core.WasmSegmenter(rate);
      segmenter.set_variant(mediaVariant);
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

// Anything the page sent while this worker was still loading.
for (const e of early.splice(0)) void onmessage(e);
