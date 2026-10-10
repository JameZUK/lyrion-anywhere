// Main thread: sign-in, AudioContext lifecycle, and wiring the worker to the AudioWorklet.

import { AuthError, account, groupKey, login, logout, refresh, totpConfirm, totpDisable, totpSetup } from './auth.js';

const $ = (id) => document.getElementById(id);
const logEl = $('log');
const log = (msg) => {
  logEl.textContent = `${new Date().toLocaleTimeString('en-GB')}  ${msg}\n` + logEl.textContent;
};

// Refresh the 15-minute access token well before it expires.
const REFRESH_EVERY_MS = 10 * 60 * 1000;

let session = null; // {access_token, user, device}
let refreshTimer = null;
let worker = null;
let ctx = null;
let node = null;

function show(panel) {
  for (const id of ['login-form', 'start-panel', 'player']) $(id).hidden = id !== panel;
}

function signedIn(s) {
  session = s;
  $('who').textContent = `${s.user.username} (${s.device.name})`;
  clearInterval(refreshTimer);
  refreshTimer = setInterval(renewToken, REFRESH_EVERY_MS);
}

async function renewToken() {
  try {
    signedIn(await refresh());
    if (worker) worker.postMessage({ cmd: 'token', token: session.access_token });
    return true;
  } catch (err) {
    log(`session expired: ${err.message}`);
    signedOut();
    return false;
  }
}

function signedOut() {
  session = null;
  clearInterval(refreshTimer);
  if (worker) {
    worker.terminate();
    worker = null;
  }
  if (ctx) ctx.close();
  ctx = null;
  show('login-form');
}

// iOS/Safari 16.4+: declare this page a media player so audio keeps playing with the
// screen locked and ignores the ring/silent switch. Feature-detected; a no-op elsewhere.
function configureAudioSession() {
  if (navigator.audioSession) {
    try {
      navigator.audioSession.type = 'playback';
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

$('login-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  $('login').disabled = true;
  try {
    const code = $('totp-row').hidden ? undefined : $('totp').value.trim();
    signedIn(await login($('username').value.trim(), $('password').value, code));
    $('password').value = '';
    $('totp').value = '';
    $('totp-row').hidden = true;
    show('start-panel');
    refreshSecurity();
  } catch (err) {
    if (err instanceof AuthError && err.totpRequired) {
      $('totp-row').hidden = false;
      $('totp').focus();
      log('enter the code from your authenticator app');
    } else {
      log(err instanceof AuthError && err.status === 429 ? 'too many attempts: wait and try again' : `sign-in failed: ${err.message}`);
    }
  } finally {
    $('login').disabled = false;
  }
});

// --- two-step verification ---------------------------------------------------

async function refreshSecurity() {
  if (!session) return;
  try {
    const a = await account(session.access_token);
    $('totp-status').textContent = a.totp_enabled ? 'on' : 'off';
    $('totp-off').hidden = a.totp_enabled;
    $('totp-disable-form').hidden = !a.totp_enabled;
    $('totp-setup').hidden = true;
  } catch (err) {
    log(`could not load account: ${err.message}`);
  }
}

$('totp-begin').addEventListener('click', async () => {
  try {
    const setup = await totpSetup(session.access_token);
    $('totp-qr').src = `data:image/svg+xml;base64,${btoa(setup.qr_svg)}`;
    $('totp-secret').textContent = groupKey(setup.secret);
    $('totp-off').hidden = true;
    $('totp-setup').hidden = false;
    $('totp-confirm-code').focus();
  } catch (err) {
    log(`could not start setup: ${err.message}`);
  }
});

$('totp-confirm-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  try {
    await totpConfirm(session.access_token, $('totp-confirm-code').value.trim());
    $('totp-confirm-code').value = '';
    $('totp-qr').removeAttribute('src');
    $('totp-secret').textContent = '';
    log('two-step verification is on');
    refreshSecurity();
  } catch (err) {
    log(err.message);
  }
});

$('totp-disable-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  try {
    await totpDisable(session.access_token, $('totp-off-password').value, $('totp-off-code').value.trim());
    log('two-step verification is off');
    refreshSecurity();
  } catch (err) {
    log(err.status === 401 ? 'password or code not right' : err.message);
  } finally {
    $('totp-off-password').value = '';
    $('totp-off-code').value = '';
  }
});

async function signOut() {
  try {
    if (session) await logout(session.access_token);
  } catch {
    /* signing out anyway */
  }
  signedOut();
}
$('logout').addEventListener('click', signOut);
$('logout-player').addEventListener('click', signOut);

$('start').addEventListener('click', async () => {
  $('start').disabled = true;
  try {
    // Must happen inside the click so the browser allows audio.
    configureAudioSession();
    const port = await createAudio(44100);
    worker = new Worker('player-worker.js', { type: 'module' });
    worker.onmessage = onWorkerMessage;
    worker.onerror = (err) => log(`worker error: ${err.message}`);
    worker.postMessage({ cmd: 'start', token: session.access_token, port }, [port]);
    worker.postMessage({ cmd: 'rate-ready', rate: 44100 });
    show('player');
  } catch (err) {
    log(`could not start: ${err.message}`);
    $('start').disabled = false;
  }
});

async function onWorkerMessage(e) {
  const m = e.data;
  if (m.log) log(m.log);
  if (m.connection) $('s-conn').textContent = m.connection;
  if (m.authFailed) {
    // The access token was refused (expired): renew it and let the worker carry on.
    if (await renewToken()) worker.postMessage({ cmd: 'token', token: session.access_token, reconnect: true });
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

// A returning browser is signed in silently through its refresh cookie.
(async () => {
  try {
    signedIn(await refresh());
    show('start-panel');
    refreshSecurity();
  } catch {
    show('login-form');
  }
})();
