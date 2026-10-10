// Main thread: UI, AudioContext lifecycle, and wiring the worker to the AudioWorklet.

const $ = (id) => document.getElementById(id);
const logEl = $('log');
const log = (msg) => {
  logEl.textContent = `${new Date().toLocaleTimeString('en-GB')}  ${msg}\n` + logEl.textContent;
};

let worker = null;
let ctx = null;
let node = null;

// iOS/Safari 16.4+: declare this page a media player so audio keeps playing with the
// screen locked and ignores the ring/silent switch. Feature-detected; a no-op elsewhere.
function configureAudioSession() {
  if (navigator.audioSession) {
    try {
      navigator.audioSession.type = 'playback';
      log(`audio session: ${navigator.audioSession.type}`);
    } catch (err) {
      log(`audio session not configurable: ${err.message}`);
    }
  }
  if ('mediaSession' in navigator && 'MediaMetadata' in window) {
    navigator.mediaSession.metadata = new MediaMetadata({ title: 'Lyrion Anywhere', artist: 'Browser player' });
  }
}

async function createAudio(rate) {
  if (ctx) await ctx.close();
  // The core outputs at the track's own rate; the browser resamples to the hardware.
  ctx = new AudioContext({ sampleRate: rate, latencyHint: 'playback' });
  await ctx.audioWorklet.addModule('worklet.js');
  node = new AudioWorkletNode(ctx, 'anywhere-sink', { outputChannelCount: [2] });
  node.connect(ctx.destination);
  const channel = new MessageChannel();
  node.port.postMessage({ port: channel.port1 }, [channel.port1]);
  await ctx.resume();
  $('s-ctx').textContent = `${ctx.sampleRate} Hz, ${ctx.state}`;
  return channel.port2;
}

$('connect-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  $('connect').disabled = true;
  const token = $('token').value.trim();
  $('token').value = '';
  try {
    // Must happen inside the click so the browser allows audio.
    configureAudioSession();
    const port = await createAudio(44100);
    worker = new Worker('player-worker.js', { type: 'module' });
    worker.onmessage = onWorkerMessage;
    worker.onerror = (err) => log(`worker error: ${err.message}`);
    worker.postMessage({ cmd: 'start', token, port }, [port]);
    worker.postMessage({ cmd: 'rate-ready', rate: 44100 });
    $('connect-form').hidden = true;
    $('player').hidden = false;
  } catch (err) {
    log(`could not start: ${err.message}`);
    $('connect').disabled = false;
  }
});

async function onWorkerMessage(e) {
  const m = e.data;
  if (m.log) log(m.log);
  if (m.closed) {
    $('s-output').textContent = 'disconnected';
  }
  if (m.rate) {
    log(`switching audio output to ${m.rate} Hz`);
    const port = await createAudio(m.rate);
    // The worker must route PCM to the new worklet before pulling at the new rate.
    worker.postMessage({ cmd: 'start-sink', port }, [port]);
    worker.postMessage({ cmd: 'rate-ready', rate: m.rate });
  }
  if (m.status) {
    const s = m.status;
    $('s-name').textContent = s.name;
    $('s-output').textContent = s.output + (s.last_error ? ` (last error: ${s.last_error})` : '');
    $('s-elapsed').textContent = `${(s.elapsed_ms / 1000).toFixed(1)} s`;
    $('s-buffered').textContent = `${(s.buffered_ms / 1000).toFixed(1)} s PCM, ${Math.round(s.input_bytes / 1024)} KiB compressed`;
    $('s-rate').textContent = s.sample_rate ? `${s.sample_rate} Hz` : '–';
    $('s-volume').textContent = s.volume[0].toFixed(2);
    if (ctx) $('s-ctx').textContent = `${ctx.sampleRate} Hz, ${ctx.state}`;
  }
}
