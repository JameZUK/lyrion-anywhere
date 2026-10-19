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
const { AuthWait, CLOSE, FlushWatch, Outbox, RateRequest, SeekTimer, backoffMs, closeAction, describeSeek, downloadStream, loadCore, qualityMessage, routeMessage, sanitiseQuality, signInMessage, socketSilent } =
  await import(`./reconnect.js${VQ}`);
const { Presentation } = await import(`./presentation.js${VQ}`);
const { VisualTap } = await import(`./visual.js${VQ}`);
const { planTrim } = await import(`./trim.js${VQ}`);
const { LocalHold, effectiveOutput, parseLocal } = await import(`./local.js${VQ}`);
const { MediaCorrections } = await import(`./media-sync.js${VQ}`);
const { Persisting, describeDiag, describeDrained, describeNotConsuming, describeOutputChange, outputCause, outputName, secs } = await import(`./diag.js${VQ}`);

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
// Whether the gateway reports LMS being busy and sends text keepalives on request (null until known).
let busyFeature = null;
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
// A pause made on this device while the server is out of reach (see local.js): the output
// is held, and the core and what it tells LMS are left alone.
const localHold = new LocalHold();
// The stream being fetched has been received to its end (the page's "On this device").
let streamReceived = false;

// Output mode: 'worklet' (PCM to an AudioWorklet) or 'media' (fMP4/FLAC segments for an
// <audio> element on the main thread, which keeps playing in the background on iOS).
let outputMode = 'worklet';
// Seconds of audio to keep queued in the media element: enough to ride out timer throttling
// while the page is in the background, small enough that buffers stay modest.
//
// It is also how late LMS's sync corrections are heard in this mode: they are made in the
// audio not yet appended (see media-sync.js), so the element plays this much first. They are
// reported to the core at once, so LMS doesn't repeat them meanwhile; the group is only
// audibly out of step for these seconds after a correction. Kept as it is: a shorter lead
// risks the element running dry when iOS throttles a page in the background, which is the
// reason this mode exists.
const MEDIA_AHEAD_SECS = 8;
let segmenter = null;
let mediaVariant = 0; // init-segment variant the browser's self-test accepted
let mediaWaiting = false; // flushed; waiting for the main thread's new timeline position
// LMS's sync skips and pauses, made in the audio before muxing (see media-sync.js).
const mediaCorrections = new MediaCorrections();
// The visualiser (Wave 2 §4.1): analyses the PCM going out, only while the page shows it.
const visual = new VisualTap((rate, channels) => new core.WasmVisualiser(rate, channels));

const now = () => performance.now();
// Absolute time (ms), comparable with the page's: each has its own performance.now().
const absNow = () => performance.timeOrigin + performance.now();
const log = (msg) => postMessage({ log: msg });
// A line the page logs at most once per few seconds for each `kind` (see LogLimiter).
const logAs = (msg, kind) => postMessage({ log: msg, kind });

// Diagnostics (issue #12): the last `strm` from LMS ({ command, at }), the output state last
// logged, and whether the media element has stopped taking audio.
let lastStrm = null;
let loggedOutput = '';
// Media mode: when the element last reported frames played (or the output last began playing).
let lastFramesAt = 0;
// The element is playing with a full lead, but reports nothing played, for this long (ms).
const NOT_CONSUMING_MS = 5000;
const notConsuming = new Persisting(0);
let notConsumingFrom = 0;
// The decoded audio the core holds (ms), from its status.
let coreBufferedMs = 0;

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
  visual.flush();
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
  visual.flush();
  lastFramesAt = now();
  notConsuming.reset();
  epoch++;
  presentation.dropRuns();
  timedStart = false;
  localHold.onFlush(now());
  mediaCorrections.reset();
  if (outputMode === 'media') {
    if (segmenter) {
      mediaWaiting = true;
      postMessage({ media: { flush: true } });
    }
  } else if (sink) {
    sink.postMessage({ flush: true, epoch });
  }
}

/**
 * LMS dropped the tracks after the audible one (`strm f`: the next track was deleted or
 * moved), which keeps playing. Drop what the output already holds of the later tracks.
 */
function trimOutput(t) {
  const plan = planTrim(t, { mode: outputMode, pulled, segmenter, mediaWaiting });
  if (plan.drop > 0) {
    pulled = plan.pulled;
    visual.trim(pulled);
  }
  if (plan.worklet && sink) sink.postMessage({ ...plan.worklet, epoch });
  for (const m of plan.media) postMessage({ media: m });
  log(`LMS dropped the tracks after track ${t.keep_track}; dropped ${plan.drop} frames queued`);
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
    // Learn the gateway's features first (briefly), so even the first sign-in can ask for
    // keepalives; if that fails, it is asked again after the greeting.
    await learnFeatures(3000);
    connect(false);
  } else if (msg.cmd === 'visual') {
    // The page shows the visualiser (or stopped showing it).
    visual.setOn(msg.on === true);
  } else if (msg.cmd === 'local') {
    // The page acts on this device while the server is out of reach.
    const l = parseLocal(msg);
    if (l?.hold) {
      localHold.hold();
      log('paused on this device');
    } else if (l?.release) {
      localHold.release(l.release, now());
    }
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
      if (frames > 0) {
        seekMark('audible');
        lastFramesAt = now();
      }
      played += frames;
      const at = Number.isFinite(msg.at) ? Math.min(now(), Math.max(0, msg.at - performance.timeOrigin)) : now();
      // Less what a sync pause has already told the core of (see media-sync.js).
      player.presented(mediaCorrections.report(frames), at);
    }
  } else if (msg.cmd === 'media-drained') {
    // Media mode: the element played everything appended (it is waiting for more). Count
    // it all, so the core sees the end of the playlist and LMS stops ("played out").
    // Only while playing: a drain reported around a pause or a flush is not the end.
    if (player && segmenter && !mediaWaiting && lastOutput === 'Playing') {
      if (segmenter.pending_frames() > 0) {
        // Something is still held back (a partial frame): send it; the element plays on.
        const pending = segmenter.pending_frames();
        postSegment(segmenter.flush());
        logAs(describeDrained('flushed', pending), 'media:drained');
      } else {
        // Everything pulled has been played. The core caps this at what was pulled, so it
        // can't over-count; the worker's own estimate catches up too.
        player.played(Number.MAX_SAFE_INTEGER);
        played = pulled;
        logAs(describeDrained('played-out'), 'media:drained');
      }
    } else if (player && segmenter) {
      logAs(describeDrained('ignored', mediaWaiting ? 'a flush is under way' : `output ${outputName(lastOutput)}`), 'media:drained');
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

/**
 * Ask the gateway once which features it has: whether a session may change its level, and
 * whether it sends keepalives (asked for at the next sign-in).
 */
async function learnFeatures(timeoutMs = 0) {
  if (qualityFeature !== null) return;
  const abort = new AbortController();
  const timer = timeoutMs > 0 ? setTimeout(() => abort.abort(), timeoutMs) : 0;
  try {
    const r = await fetch(new URL('/v1/me', self.location.origin), {
      headers: { Authorization: `Bearer ${token}`, 'X-Anywhere-Client': '1' },
      cache: 'no-store',
      signal: abort.signal,
    });
    if (!r.ok) return;
    const me = await r.json();
    qualityFeature = me?.features?.stream_quality === true;
    busyFeature = me?.features?.lms_busy === true;
    sendQuality();
  } catch {
    // Offline, refused or slow: asked again on the next connection.
  } finally {
    clearTimeout(timer);
  }
}

function connect(resume) {
  const url = new URL('/v1/player', self.location.origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  ws = new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => {
    ws.send(signInMessage(token, resume ? { session, secret, received } : null, quality, busyFeature === true));
  };
  ws.onmessage = (m) => {
    // Anything at all shows the socket is alive (a keepalive is only for this).
    lastHeard = now();
    routeMessage(m.data, {
      frames: onFrames,
      greeting: onGreeting,
      ignored: () => log('ignored an unexpected text message from the gateway'),
    });
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
  void learnFeatures();
  attempt = 0;
  // A fresh session (not a resumed one): what the page did here meanwhile must be retold.
  const fresh = !g.resumed || !player;
  if (fresh) {
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
    // Each strm from LMS and STAT sent, for the connection log.
    player.set_diagnostics(true);
    lastStrm = null;
    loggedOutput = '';
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
  postMessage({ connection: 'connected', fresh, player: { mac: g.mac, name: g.name } });
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
    // After the flush check: a full flush has already dropped everything.
    const trim = player.take_trim();
    if (trim) trimOutput(trim);
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
 * rather than after the audio queued ahead: in the worklet (to the frame). The media element
 * gets the timed start; its skips and pauses are made in the audio before muxing and
 * reported at once (see media-sync.js), never by seeking or pausing the element, which
 * stalls it on iPadOS for about as long as the correction gained.
 */
function applyCorrections() {
  if (!player) return;
  for (const c of player.take_corrections()) {
    if (outputMode === 'media') {
      if (c.type === 'start') postMessage({ media: { startAt: performance.timeOrigin + c.at } });
      else if (c.type === 'pause') mediaPause(c.ms);
      else if (c.type === 'skip') mediaSkip(c.ms);
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
  noteDiagnostics();
}

/** Log each strm from LMS and STAT sent since the last call (see diag.js). */
function noteDiagnostics() {
  for (const e of player.take_diagnostics()) {
    if (e.type === 'strm') lastStrm = { command: e.command, at: now() };
    const d = describeDiag(e);
    if (d) logAs(d.line, d.kind);
  }
}

/** Log a change of the output state (state changes are never rate-limited). */
function noteOutput(output, raw) {
  const name = outputName(output);
  if (name === loggedOutput) return;
  if (loggedOutput) log(describeOutputChange(loggedOutput, name, outputCause(loggedOutput, name, raw, lastStrm, now())));
  loggedOutput = name;
}

function closeStream() {
  if (stream) {
    if (stream.controller) stream.controller.abort();
    stream = null;
  }
}

function openStream(n) {
  closeStream();
  streamReceived = false;
  const id = ++streamId;
  stream = { id, n, controller: null };
  fetchStream(id, n);
}

function fetchStream(id, n) {
  return downloadStream(
    {
      player,
      current: () => stream && stream.id === id,
      fetch: (path, init) => fetch(path, init),
      session: () => session,
      token: () => token,
      setController: (c) => (stream.controller = c),
      sleep,
      now,
      log,
      logAs,
      mark: seekMark,
      ack: sendAck,
      runActions,
      renewToken,
      received: () => (streamReceived = true),
      // What the core holds of the stream, not yet decoded (only asked for the log).
      undecoded: () => JSON.parse(player.status_json()).input_bytes,
      maxTries: STREAM_RETRIES,
      ackEvery: ACK_EVERY,
    },
    n,
  );
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
  // Held here: the outputs act as if paused ('Held'); the core plays on, unheard.
  const output = effectiveOutput(status.output, localHold.update(status.output, now()));
  noteOutput(output, status.output);
  if (outputMode === 'media') {
    coreBufferedMs = status.buffered_ms;
    mediaTick(output);
  } else {
    holdWorklet(output);
    workletTick();
  }
  lastOutput = output;
  // The newest visualiser frame now being heard.
  const frame = visual.due(played, now());
  if (frame) postMessage({ visual: frame }, [frame.buffer]);
  status.connection = open ? 'connected' : 'reconnecting';
  status.held = localHold.held;
  status.received = streamReceived;
  status.track = player.pulled_track() ?? null;
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
  // A timed start releases the worklet itself, unless this device holds the output.
  if (timed && output !== 'Held') return;
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
      visual.feed(chunk, rate, pulled);
      sink.postMessage({ pcm: chunk }, [chunk.buffer]);
      seekMark('queued');
      pulled += frames;
    }
  }
}

/** Media mode: LMS's skip, as decoded audio dropped now and counted as presented. */
function mediaSkip(ms) {
  const rate = player.output_rate();
  if (mediaWaiting || !rate) return;
  const dropped = mediaCorrections.skip(ms, rate, player, now());
  pulled += dropped;
  played += dropped;
  log(`sync: skipped ${ms} ms${dropped < Math.round((ms * rate) / 1000) ? ' (the rest as it is decoded)' : ''}`);
}

/** Media mode: LMS's pause, as silence in the audio; the core is told at once. */
function mediaPause(ms) {
  const rate = player.output_rate();
  if (mediaWaiting || !segmenter || !rate || segmenter.rate() !== rate) return;
  // The silence counts as queued audio, or each pause would lengthen the lead for good.
  pulled += mediaCorrections.pause(ms, rate, segmenter, player.pulled_track());
  postSegment(segmenter.segment());
  log(`sync: paused ${ms} ms`);
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
  const paused = output.startsWith('Paused') || output === 'Loaded' || output === 'Held';
  const wasPaused = String(lastOutput).startsWith('Paused') || lastOutput === 'Loaded' || lastOutput === 'Held';
  if (paused && !wasPaused) postMessage({ media: { pause: true } });
  if (output === 'Playing' && lastOutput !== 'Playing') {
    postMessage({ media: { play: true } });
    lastFramesAt = now(); // the element is only now asked to play
  }
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
  // The rest of a sync skip that was more than the core had decoded.
  if (mediaCorrections.pendingSkip > 0) {
    const dropped = mediaCorrections.dropDue(player, now());
    pulled += dropped;
    played += dropped;
  }
  let exhausted = false;
  while (pulled - played < rate * MEDIA_AHEAD_SECS) {
    const buf = new Float32Array(4096 * 2);
    // One track at a time, so the segmenter knows where each track's audio goes (a partial
    // flush cuts the timeline there: see trimOutput).
    const frames = player.pull_one(buf);
    if (!frames) {
      exhausted = player.audio_exhausted();
      break;
    }
    const pcm = buf.subarray(0, frames * 2);
    // Kept while off: the element plays seconds behind what is pulled here.
    visual.feed(pcm, rate, pulled, true);
    segmenter.push_track(pcm, player.pulled_track());
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
  watchConsuming(output, rate);
}

/**
 * Media mode: the output is playing and the element holds all the lead it may, but has
 * reported nothing played for {@link NOT_CONSUMING_MS}: say so once (and once when it ends).
 * The worker then waits for it, as it should; the log shows the stall was the element's.
 */
function watchConsuming(output, rate) {
  const lead = (pulled - played) / rate;
  const full = lead >= MEDIA_AHEAD_SECS - 0.5;
  const t = now();
  const change = notConsuming.update(output === 'Playing' && full && t - lastFramesAt >= NOT_CONSUMING_MS, t);
  if (change === 'began') {
    notConsumingFrom = lastFramesAt;
    log(describeNotConsuming(lead, (Number(coreBufferedMs) || 0) / 1000));
  } else if (change === 'ended') {
    log(
      lastFramesAt > notConsumingFrom
        ? `media: element playing again after ${secs((t - notConsumingFrom) / 1000)} s`
        : `media: no longer waiting for the element (${outputName(output)})`,
    );
  }
}

// Anything the page sent while this worker was still loading.
for (const e of early.splice(0)) void onmessage(e);
