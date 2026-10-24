// Settings page script: pure helpers, and no HTML-parsing APIs anywhere.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const src = readFileSync(
  new URL('../../Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/anywhere.js', import.meta.url),
  'utf8',
);

test('never parses strings as HTML (names come from remote users)', () => {
  const code = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, ''); // ignore comments
  const banned = /\.\s*(?:innerHTML|outerHTML)\b|insertAdjacentHTML\s*\(|document\.write\s*\(|\beval\s*\(|new\s+Function\s*\(/;
  assert.ok(!banned.test(code), `uses an HTML-parsing API: ${code.match(banned)}`);
  // The check itself must work.
  assert.ok(banned.test('x.innerHTML = y'));
  assert.ok(banned.test('el.insertAdjacentHTML("beforeend", s)'));
});

const ctx = { __anywhereTest: {} };
ctx.globalThis = ctx;
vm.runInNewContext(src, ctx);
const t = ctx.__anywhereTest;

test('builds anywhere commands, skipping empty values', () => {
  assert.deepEqual(Array.from(t.command('grant', { username: 'alice', player: 'aa:bb:cc:dd:ee:01', id: '' })), [
    'anywhere', 'grant', 'username:alice', 'player:aa:bb:cc:dd:ee:01',
  ]);
  assert.deepEqual(Array.from(t.command('users')), ['anywhere', 'users']);
});

test('adds the CSRF token last, and never a caller-supplied one', () => {
  const tok = 'ab'.repeat(16);
  assert.deepEqual(Array.from(t.command('user_delete', { username: 'alice' }, tok)), [
    'anywhere', 'user_delete', 'username:alice', 'csrf:' + tok,
  ]);
  assert.deepEqual(Array.from(t.command('user_delete', { username: 'alice', csrf: 'x' }, tok)), [
    'anywhere', 'user_delete', 'username:alice', 'csrf:' + tok,
  ]);
  assert.deepEqual(Array.from(t.command('restart', {}, '')), ['anywhere', 'restart']);
});

test('reads the token only from the page, and only if well-formed', () => {
  const doc = (v) => ({ getElementById: (id) => (id === 'anywhere-app' ? { getAttribute: () => v } : null) });
  assert.equal(t.pageToken(doc('0123456789abcdef0123456789abcdef')), '0123456789abcdef0123456789abcdef');
  for (const bad of [null, '', 'x', '0123456789abcdef0123456789abcdef0', '0123456789ABCDEF0123456789ABCDEF', 'a b']) {
    assert.equal(t.pageToken(doc(bad)), '', String(bad));
  }
  assert.equal(t.pageToken({ getElementById: () => null }), '');
});

test('validates input like the gateway does', () => {
  assert.ok(t.validUsername('alice.b-1'));
  for (const bad of ['', 'Alice', 'a b', 'a/b', 'x'.repeat(65)]) assert.ok(!t.validUsername(bad), bad);
  assert.ok(t.validPassword('ten chars!'));
  assert.ok(!t.validPassword('short'));
  assert.ok(!t.validPassword('long enough\nbut broken'));
});

test('formats times and durations', () => {
  assert.equal(t.formatTime(0), '\u2013');
  assert.equal(t.formatUptime(0), '\u2013');
  assert.equal(t.formatUptime(125), '2 min');
  assert.equal(t.formatUptime(3725), '1 h 2 min');
});

// --- the whole page, rendered into a minimal DOM with hostile data -----------------------

/** A tiny DOM: text is only ever text, and any attempt to parse HTML throws. */
function fakeDom() {
  const created = [];
  const focused = [];
  class Node {
    constructor(tag) {
      this.tagName = tag.toUpperCase();
      this.children = [];
      this.attrs = {};
      this.listeners = {};
      this._text = '';
      this.value = '';
      this.className = '';
      this.classList = { add() {}, remove() {} };
      created.push(this);
    }
    set textContent(v) { this._text = String(v); this.children = []; }
    get textContent() { return this._text + this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join(''); }
    set innerHTML(_) { throw new Error('innerHTML used'); }
    set outerHTML(_) { throw new Error('outerHTML used'); }
    insertAdjacentHTML() { throw new Error('insertAdjacentHTML used'); }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
    removeAttribute(k) { delete this.attrs[k]; }
    hasAttribute(k) { return k in this.attrs; }
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    append(...c) { this.children.push(...c); }
    replaceChildren(...c) { this._text = ''; this.children = c; }
    prepend(...c) { this.children.unshift(...c); }
    after() {}
    contains(n) { return n === this || this.children.some((c) => typeof c !== 'string' && c.contains(n)); }
    closest() { return null; }
    get isConnected() { return !this.detached; }
    focus() { focused.push(this); }
    click() { for (const fn of this.listeners.click || []) fn({ target: this }); }
  }
  const byId = new Map();
  for (const id of ['anywhere-app', 'aw-status', 'aw-users', 'aw-pair', 'aw-devices', 'aw-audit', 'aw-message', 'aw-create-btn', 'aw-new-user', 'aw-new-password', 'aw-new-role', 'aw-session-days', 'aw-session-btn', 'aw-plugins', 'aw-plugins-btn', 'aw-offline', 'aw-create', 'aw-session', 'aw-plugins-box', 'aw-h-status', 'aw-h-users', 'aw-h-signin', 'aw-h-plugins', 'aw-h-devices', 'aw-h-audit']) {
    byId.set(id, new Node('div'));
  }
  const docListeners = {};
  const document = {
    createElement: (tag) => new Node(tag),
    getElementById: (id) => byId.get(id),
    activeElement: null,
    body: null,
    querySelectorAll: () => created.filter((e) => 'data-aw-key' in e.attrs),
    addEventListener: (type, fn) => { docListeners[type] = fn; },
    write() { throw new Error('document.write used'); },
  };
  return { document, byId, created, focused, ready: () => docListeners.DOMContentLoaded() };
}

const HOSTILE = '<img src=x onerror=document.title="PWNED">';

test('renders remote-controlled names as text, never as markup', async () => {
  const dom = fakeDom();
  const replies = {
    status: { running: 1, enabled: 1, pid: 42, uptime: 60, listen: '127.0.0.1:9443', binary_ok: 1, binary: '/x', data_dir: '/d' },
    players: { players_loop: [{ playerid: 'aa:bb:cc:dd:ee:01', name: HOSTILE }] },
    users: { data: { users: [{ username: 'bob', role: 'listener', totp_enabled: 0, disabled: 0, downloads: true, download_formats: ['original', 'opus', HOSTILE] }] } },
    grants: { data: { players: ['aa:bb:cc:dd:ee:01'] } },
    devices: { data: { devices: [{ id: 1, name: HOSTILE, username: 'bob', mac: '02:4c:41:00:00:09', last_seen: 1, last_addr: '198.51.100.7', client_kind: 'pwa', client_platform: HOSTILE, user_agent: HOSTILE }] } },
    audit: { data: { audit: [{ at: 1, event: 'login', username: 'bob', detail: HOSTILE }] } },
    settings: { data: { session_days: 45 } },
    plugins: { data: { plugins: [
      { command: 'audiomuseai', name: HOSTILE, kind: 'menu', admin_only: true },
      { command: 'spotty', name: 'Spotty', kind: 'app', admin_only: false },
    ] } },
  };
  const TOKEN = 'cd'.repeat(16);
  dom.byId.get('anywhere-app').setAttribute('data-csrf', TOKEN);
  const sent = [];
  const fetch = async (_url, opts) => {
    const cmd = JSON.parse(opts.body).params[1];
    sent.push(cmd);
    const key = cmd[0] === 'players' ? 'players' : cmd[1];
    return { json: async () => ({ result: replies[key] || {} }) };
  };
  const ctx = { document: dom.document, fetch, setInterval: () => 0, setTimeout: () => 0, Map, Promise, JSON, Date, Math, Object, String, Error };
  ctx.globalThis = ctx;
  vm.runInNewContext(src, ctx);
  dom.ready();
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); // let the async views settle

  assert.ok(dom.byId.get('aw-devices').textContent.includes(HOSTILE), 'device name shown verbatim as text');
  assert.ok(dom.byId.get('aw-audit').textContent.includes(HOSTILE), 'audit detail shown verbatim as text');
  assert.ok(dom.byId.get('aw-users').textContent.includes(HOSTILE), 'granted player name shown verbatim as text');
  assert.equal(dom.created.filter((e) => e.tagName === 'IMG' || e.tagName === 'SCRIPT').length, 0, 'no img/script elements');
  for (const e of dom.created) {
    for (const k of Object.keys(e.attrs)) assert.ok(!/^on/i.test(k), `event-handler attribute ${k}`);
  }
  assert.equal(dom.byId.get('aw-message').textContent, '', 'no error reported');
  assert.equal(dom.byId.get('aw-session-days').value, '45', 'the sign-in lifetime is shown');
  assert.ok(dom.byId.get('aw-plugins').textContent.includes(HOSTILE), 'plugin names shown verbatim as text');

  assert.ok(dom.byId.get('aw-users').textContent.includes('on (original, Opus)'), 'download permission shown, unknown formats dropped');

  // The downloads toggle sends the change with the token.
  const toggle = dom.created.find((e) => e.tagName === 'BUTTON' && e.textContent === 'Turn off downloads');
  assert.ok(toggle, 'downloads toggle shown');
  for (const fn of toggle.listeners.click) fn();
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
  const off = sent.find((c) => c[0] === 'anywhere' && c[1] === 'user_downloads');
  assert.deepEqual(off, ['anywhere', 'user_downloads', 'username:bob', 'enabled:0', 'csrf:' + TOKEN]);

  // Changing something carries the page's token.
  dom.byId.get('aw-session-days').value = '60';
  for (const fn of dom.byId.get('aw-session-btn').listeners.click) fn();
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
  const change = sent.find((c) => c[0] === 'anywhere' && c[1] === 'session_days');
  assert.ok(change, 'session_days sent');
  assert.deepEqual(change, ['anywhere', 'session_days', 'days:60', 'csrf:' + TOKEN]);
  // Devices show the client type, address and browser, as text.
  const devices = dom.byId.get('aw-devices').textContent;
  assert.ok(devices.includes('198.51.100.7'), 'last address shown');
  assert.ok(devices.includes('Installed web app · ' + HOSTILE), 'client type shown, platform as text');

  // Renaming: an input in place of the name, sent with the token; bad names stay local.
  const rename = dom.created.find((e) => e.tagName === 'BUTTON' && e.textContent === 'Rename');
  assert.ok(rename, 'rename button shown');
  for (const fn of rename.listeners.click) fn();
  const input = dom.created.find((e) => e.tagName === 'INPUT' && String(e.attrs['aria-label']).startsWith('New name for '));
  assert.ok(input, 'name input shown');
  assert.equal(input.value, HOSTILE, 'starts from the current name, as a value');
  const save = dom.created.filter((e) => e.tagName === 'BUTTON' && e.textContent === 'Save').pop();
  input.value = '   ';
  for (const fn of save.listeners.click) fn();
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
  assert.ok(!sent.some((c) => c[1] === 'device_rename'), 'a blank name is not sent');
  assert.match(dom.byId.get('aw-message').textContent, /1 to 64/);
  input.value = ' Kitchen iPad ';
  for (const fn of save.listeners.click) fn();
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
  assert.deepEqual(sent.find((c) => c[1] === 'device_rename'), ['anywhere', 'device_rename', 'id:1', 'name:Kitchen iPad', 'csrf:' + TOKEN]);

  // status too: without the token LMS leaves out the paths, pid and stderr (review 2026-10-15).
  assert.ok(sent.some((c) => c[0] === 'anywhere' && c[1] === 'status'), 'status asked for');
  for (const c of sent.filter((c) => c[0] === 'anywhere')) {
    assert.equal(c[c.length - 1], 'csrf:' + TOKEN, `token sent with ${c[1]}`);
  }
});

test('the script is ASCII only (non-ASCII text is written as escapes)', () => {
  assert.ok(!/[^\x00-\x7f]/.test(src), `non-ASCII at ${src.search(/[^\x00-\x7f]/)}`);
  // ...and the escapes produce the intended characters.
  assert.equal(t.formatTime(0), '\u2013');
});

test('the sign-in lifetime shown: the gateway value, its default, or 90', () => {
  assert.equal(t.DEFAULT_SESSION_DAYS, 90);
  assert.equal(t.sessionDaysShown({ session_days: 45 }), 45);
  assert.equal(t.sessionDaysShown({ session_days: '45' }), 45, 'LMS may pass numbers on as strings');
  assert.equal(t.sessionDaysShown({ session_days: 0, session_days_default: 60 }), 60);
  for (const s of [{}, null, undefined, { session_days: '' }, { session_days: 'x' }, { session_days: 400 }, { session_days: -3 }]) {
    assert.equal(t.sessionDaysShown(s), 90, JSON.stringify(s));
  }
});

test('device names: 1 to 64 characters, trimmed, no control characters', () => {
  assert.equal(t.validDeviceName('  Kitchen iPad '), 'Kitchen iPad');
  assert.equal(t.validDeviceName('é'.repeat(64)), 'é'.repeat(64));
  assert.equal(t.validDeviceName('\u{1f3b5}'.repeat(64)), '\u{1f3b5}'.repeat(64), 'counts characters, not UTF-16 units');
  for (const bad of ['', '   ', 'x'.repeat(65), 'a\nb', 'a\u0000b', 'a\u007fb', 'a\u0085b', null, undefined, 42, ['x']]) {
    assert.equal(t.validDeviceName(bad), null, JSON.stringify(bad));
  }
});

test('client type text', () => {
  assert.equal(t.clientText({ client_kind: 'pwa', client_platform: 'Safari on iPadOS' }), 'Installed web app · Safari on iPadOS');
  assert.equal(t.clientText({ client_kind: 'browser', client_platform: 'Chrome on Android' }), 'Browser tab · Chrome on Android');
  assert.equal(t.clientText({ client_kind: 'android', client_platform: 'Pixel 8' }), 'Android app · Pixel 8');
  assert.equal(t.clientText({ client_kind: 'android' }), 'Android app');
  assert.equal(t.clientText({}), 'unknown');
  assert.equal(t.clientText(null), 'unknown');
  assert.equal(t.clientText({ client_kind: '__proto__' }), 'unknown');
  assert.equal(t.clientText({ client_kind: 'toString', client_platform: 5 }), 'unknown');
});

test('diagnostics are shown as plain JSON text', () => {
  const text = t.diagnosticsText({ updated_at: 0, report: { playback: { mode: 'media' }, log: ['<img src=x onerror=alert(1)>'] } });
  assert.ok(text.includes('"mode": "media"'));
  assert.ok(text.includes('<img src=x'), 'kept as text (the page sets it with textContent)');
  assert.equal(t.diagnosticsText(null), 'null');
});

test('sign-in lifetime: whole days from 1 to 365 only', () => {
  assert.equal(t.validDays('30'), 30);
  assert.equal(t.validDays(' 365 '), 365);
  assert.equal(t.validDays(1), 1);
  for (const bad of ['', '0', '366', '-1', '1.5', '1e2', 'abc', '30x', null, undefined, '0x10']) {
    assert.equal(t.validDays(bad), null, String(bad));
  }
});

test('admin-only plugins: only ticked, well-formed commands, sorted', () => {
  assert.equal(t.adminOnlyArg([
    { command: 'spotty', checked: false },
    { command: 'audiomuseai', checked: true },
    { command: 'custombrowse', checked: true },
    { command: 'bad word', checked: true },
    { command: 'x,y', checked: true },
  ]), 'audiomuseai,custombrowse');
  assert.equal(t.adminOnlyArg([]), '');
  // A plugin with several commands is reserved as a whole.
  assert.equal(t.adminOnlyArg([
    { commands: ['randomplay', 'randomplaygenrelist', 'randomplaylibrarylist'], checked: true },
    { commands: ['spotty'], checked: false },
  ]), 'randomplay,randomplaygenrelist,randomplaylibrarylist');
});

test('download permission text', () => {
  assert.equal(t.downloadsText({ downloads: false, download_formats: ['original'] }), 'off');
  assert.equal(t.downloadsText({ downloads: true, download_formats: ['original', 'mp3', 'opus'] }), 'on (original, MP3, Opus)');
  assert.equal(t.downloadsText({ downloads: true, download_formats: [] }), 'on');
  assert.equal(t.downloadsText({ downloads: 1 }), 'off', 'only a real true counts');
  assert.equal(t.downloadsText({ downloads: true, download_formats: ['__proto__', 'constructor', 'toString'] }), 'on');
  assert.equal(t.downloadsText(null), 'off');
});

// --- page behaviour: stopped gateway, restart, Enter, grants, labels, focus, audit -------

const qrSrc = readFileSync(
  new URL('../../Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/qr.js', import.meta.url),
  'utf8',
);

test('audit events in words; unknown codes as they are', () => {
  assert.equal(t.auditEventText('login.failed'), 'Sign-in failed');
  assert.equal(t.auditEventText('pairing.ok'), 'Device added with a code');
  assert.equal(t.auditEventText('user.enabled'), 'User enabled');
  assert.equal(t.auditEventText('something.new'), 'something.new');
  assert.equal(t.auditEventText('__proto__'), '__proto__');
  assert.equal(t.auditEventText('toString'), 'toString');
  assert.equal(t.auditEventText(HOSTILE), HOSTILE, 'shown as text by the page');
  assert.equal(t.auditEventText(undefined), '');
});

test('http:// public URLs are spotted', () => {
  for (const u of ['http://music.example.com', ' HTTP://music.example.com', 'http://10.0.0.10:9443']) assert.ok(t.isHttpUrl(u), u);
  for (const u of ['https://music.example.com', '', null, 'music.example.com', 'httpx://a']) assert.ok(!t.isHttpUrl(u), String(u));
});

test('Enter, but not while composing text', () => {
  assert.ok(t.isEnter({ key: 'Enter' }));
  assert.ok(!t.isEnter({ key: 'Enter', isComposing: true }));
  assert.ok(!t.isEnter({ key: 'Enter', keyCode: 229 }));
  assert.ok(!t.isEnter({ key: 'a' }));
  assert.ok(!t.isEnter(null));
});

test('restart and stopped-gateway messages', () => {
  assert.equal(t.restartText({ started: 1 }), 'Gateway restarted');
  assert.equal(t.restartText({}), 'Gateway restarted');
  assert.match(t.restartText({ started: 0, message: 'Remote access is turned off, so...' }), /turned off/);
  assert.match(t.restartText({ started: '0' }), /turned off/);
  assert.equal(t.offlineText({ running: 1, enabled: 1 }), null);
  assert.match(t.offlineText({ running: 0, enabled: 0 }), /turned off/);
  assert.match(t.offlineText({ running: 0, enabled: 1 }), /not running/);
  assert.match(t.offlineText(null), /not running/);
});

/** The page over the fake DOM with the given replies; returns helpers to drive it. */
async function page(replies) {
  const dom = fakeDom();
  const TOKEN = 'ab'.repeat(16);
  dom.byId.get('anywhere-app').setAttribute('data-csrf', TOKEN);
  dom.byId.get('anywhere-app').setAttribute('data-public-url', 'https://music.example.com');
  dom.byId.get('aw-pair').setAttribute('hidden', '');
  dom.document.createElementNS = (_ns, tag) => dom.document.createElement(tag);
  const sent = [];
  const fetch = async (_url, opts) => {
    const cmd = JSON.parse(opts.body).params[1];
    sent.push(cmd);
    const key = cmd[0] === 'players' ? 'players' : cmd[1];
    const r = replies[key];
    return { json: async () => ({ result: (typeof r === 'function' ? r(cmd) : r) || {} }) };
  };
  const ctx = { document: dom.document, fetch, URL, setInterval: () => 0, setTimeout: () => 0, clearTimeout: () => {}, clearInterval: () => {}, Map, WeakMap, Set, Promise, JSON, Date, Math, Object, String, Error };
  ctx.globalThis = ctx;
  vm.runInNewContext(qrSrc, ctx);
  vm.runInNewContext(src, ctx);
  dom.ready();
  const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setImmediate(r)); };
  await settle();
  const button = (text) => dom.created.filter((e) => e.tagName === 'BUTTON' && e.textContent === text).pop();
  const click = async (b) => { for (const fn of b.listeners.click || []) fn(); await settle(); };
  return { dom, sent, TOKEN, settle, button, click };
}

const RUNNING = { running: 1, enabled: 1, pid: 42, uptime: 60, listen: '127.0.0.1:9443', binary_ok: 1, binary: '/x', data_dir: '/d' };
const BASE = {
  status: RUNNING,
  players: { players_loop: [{ playerid: 'aa:bb:cc:dd:ee:01', name: 'Kitchen' }, { playerid: 'aa:bb:cc:dd:ee:02', name: 'Lounge' }] },
  users: { data: { users: [
    { username: 'bob', role: 'listener', totp_enabled: 1, disabled: 0, downloads: false },
    { username: 'carol', role: 'listener', totp_enabled: 0, disabled: 0, downloads: true },
  ] } },
  grants: (cmd) => ({ data: { players: cmd.includes('username:bob') ? ['aa:bb:cc:dd:ee:01'] : [] } }),
  devices: { data: { devices: [
    { id: 1, name: 'Kitchen phone', username: 'bob', mac: '02:4c:41:00:00:01', last_seen: 1 },
    { id: 2, name: 'Work laptop', username: 'carol', mac: '02:4c:41:00:00:02', last_seen: 1 },
  ] } },
  audit: { data: { audit: [{ at: 1, event: 'login.failed', username: 'bob', detail: '' }] } },
  settings: { data: { session_days: 90 } },
  plugins: { data: { plugins: [] } },
};

test('a stopped gateway: its controls are off, and the page says why once', async () => {
  for (const [status, why] of [[{ running: 0, enabled: 1 }, /not running/], [{ running: 0, enabled: 0 }, /turned off/]]) {
    const p = await page({ ...BASE, status });
    for (const id of ['aw-create', 'aw-session', 'aw-plugins-box']) assert.equal(p.dom.byId.get(id).disabled, true, id);
    const note = p.dom.byId.get('aw-offline');
    assert.match(note.textContent, why);
    assert.equal(note.getAttribute('hidden'), null, 'the note is shown');
    for (const id of ['aw-users', 'aw-devices', 'aw-audit']) assert.equal(p.dom.byId.get(id).textContent, '', `${id} left empty, not repeating the note`);
    assert.deepEqual(p.sent.filter((c) => c[0] !== 'anywhere' || c[1] !== 'status'), [], 'nothing but status asked for');
    assert.equal(p.button('Restart gateway').disabled, !status.enabled, 'restart is off while remote access is off');
  }
  const p = await page(BASE);
  for (const id of ['aw-create', 'aw-session', 'aw-plugins-box']) assert.equal(p.dom.byId.get(id).disabled, false, id);
  assert.equal(p.dom.byId.get('aw-offline').getAttribute('hidden'), '', 'no note while running');
});

test('settings changed outside the page and reverted are reported in the status, as text (M1)', async () => {
  const notice = 'Settings changed outside the Lyrion Anywhere settings page were reverted (listen). <b>x</b>';
  const p = await page({ ...BASE, status: { ...RUNNING, settings_notice: notice } });
  const text = p.dom.byId.get('aw-status').textContent;
  assert.ok(text.includes(notice), 'the notice is shown verbatim, as text');
  const quiet = await page({ ...BASE, status: { ...RUNNING, settings_notice: '' } });
  assert.doesNotMatch(quiet.dom.byId.get('aw-status').textContent, /reverted/, 'nothing shown without a notice');
});

test('restart warns first and needs a second click; a turned-off gateway is reported', async () => {
  let started = 1;
  const p = await page({ ...BASE, restart: () => (started ? { started: 1 } : { started: 0, message: 'Remote access is turned off, so the gateway was not started.' }) });
  const b = p.button('Restart gateway');
  assert.match(p.dom.byId.get('aw-status').textContent, /disconnects everyone listening remotely/);
  await p.click(b);
  assert.ok(!p.sent.some((c) => c[1] === 'restart'), 'first click only asks for confirmation');
  assert.equal(b.textContent, 'Confirm restart');
  await p.click(b);
  assert.deepEqual(p.sent.find((c) => c[1] === 'restart'), ['anywhere', 'restart', 'csrf:' + p.TOKEN]);
  assert.equal(p.dom.byId.get('aw-message').textContent, 'Gateway restarted');
  started = 0;
  await p.click(b);
  await p.click(b);
  assert.match(p.dom.byId.get('aw-message').textContent, /turned off/);
});

test('Enter in the section runs its own action and never submits the LMS form', async () => {
  const p = await page(BASE);
  const keydown = p.dom.byId.get('anywhere-app').listeners.keydown[0];
  const user = p.dom.byId.get('aw-new-user');
  const pass = p.dom.byId.get('aw-new-password');
  user.tagName = 'INPUT';
  pass.tagName = 'INPUT';
  p.dom.byId.get('aw-new-role').value = 'listener';
  user.value = 'dave';
  pass.value = 'a long enough password';
  let prevented = 0;
  keydown({ key: 'Enter', target: pass, preventDefault: () => prevented++ });
  await p.settle();
  assert.equal(prevented, 1, 'the form is not submitted');
  assert.deepEqual(p.sent.find((c) => c[1] === 'user_create'), ['anywhere', 'user_create', 'username:dave', 'password:a long enough password', 'role:listener', 'csrf:' + p.TOKEN]);
  // Any other field of ours: no submit, no action.
  const other = { tagName: 'INPUT' };
  const before = p.sent.length;
  keydown({ key: 'Enter', target: other, preventDefault: () => prevented++ });
  await p.settle();
  assert.equal(prevented, 2);
  assert.equal(p.sent.length, before, 'nothing sent');
  // Enter while composing text, or on a button, is left to the browser.
  keydown({ key: 'Enter', isComposing: true, target: pass, preventDefault: () => prevented++ });
  keydown({ key: 'Enter', target: { tagName: 'BUTTON' }, preventDefault: () => prevented++ });
  assert.equal(prevented, 2);
  // The sign-in lifetime field saves with Enter.
  const days = p.dom.byId.get('aw-session-days');
  days.tagName = 'INPUT';
  days.value = '30';
  keydown({ key: 'Enter', target: days, preventDefault: () => prevented++ });
  await p.settle();
  assert.deepEqual(p.sent.find((c) => c[1] === 'session_days'), ['anywhere', 'session_days', 'days:30', 'csrf:' + p.TOKEN]);
});

test('choosing a player does nothing until Add is pressed', async () => {
  const p = await page(BASE);
  const select = p.dom.created.find((e) => e.tagName === 'SELECT' && e.attrs['aria-label'] === 'Player to give bob');
  assert.ok(select, 'player list labelled with the user');
  assert.equal((select.listeners.change || []).length, 0, 'no action on change');
  select.value = 'aa:bb:cc:dd:ee:02';
  assert.ok(!p.sent.some((c) => c[1] === 'grant'));
  const add = p.dom.created.find((e) => e.tagName === 'BUTTON' && e.attrs['aria-label'] === 'Give bob the chosen player');
  await p.click(add);
  assert.deepEqual(p.sent.find((c) => c[1] === 'grant'), ['anywhere', 'grant', 'username:bob', 'player:aa:bb:cc:dd:ee:02', 'csrf:' + p.TOKEN]);
  // Nothing chosen: said so, nothing sent.
  const carolAdd = p.dom.created.filter((e) => e.tagName === 'BUTTON' && e.attrs['aria-label'] === 'Give carol the chosen player').pop();
  const n = p.sent.length;
  await p.click(carolAdd);
  assert.equal(p.sent.length, n);
  assert.match(p.dom.byId.get('aw-message').textContent, /Choose a player/);
});

test('repeated buttons have names that say what they act on', async () => {
  const p = await page(BASE);
  const keyed = p.dom.created.filter((e) => e.tagName === 'BUTTON' && 'data-aw-key' in e.attrs);
  const names = keyed.map((b) => b.attrs['aria-label']);
  assert.ok(names.every((n) => typeof n === 'string' && n.length > 0), 'every one is named');
  assert.equal(new Set(names).size, names.length, `names are unique: ${names.join(' | ')}`);
  for (const n of ['Disable bob', 'Delete: carol', 'Sign out: Kitchen phone', 'Rename Work laptop', 'Remove Kitchen from bob', 'Reset two-step: bob', 'Add a device for carol']) {
    assert.ok(names.includes(n), n);
  }
});

test('after a list is rebuilt, focus goes back to the same control', async () => {
  const p = await page(BASE);
  const old = p.dom.created.find((e) => e.tagName === 'BUTTON' && e.attrs['data-aw-key'] === 'user:bob:disable');
  p.dom.byId.get('anywhere-app').contains = () => true;
  p.dom.document.activeElement = old;
  for (const fn of old.listeners.click) fn();
  old.detached = true; // the refresh replaces the row
  p.dom.document.activeElement = null;
  await p.settle();
  const f = p.dom.focused[p.dom.focused.length - 1];
  assert.ok(f && f !== old, 'focus moved to the new copy');
  assert.equal(f.attrs['data-aw-key'], 'user:bob:disable');
});

test('audit events are shown in words, with the code kept as a tooltip', async () => {
  const p = await page(BASE);
  const audit = p.dom.byId.get('aw-audit').textContent;
  assert.ok(audit.includes('Sign-in failed'));
  assert.ok(p.dom.created.some((e) => e.tagName === 'TD' && e.attrs.title === 'login.failed'));
});

test('the device panel tells browser users to sign in with a password', async () => {
  const p = await page({ ...BASE, pairing: { ok: 1, data: { code: 'ABCDEFGHJK', expires_in: 600 } } });
  await p.click(p.dom.created.find((e) => e.tagName === 'BUTTON' && e.attrs['aria-label'] === 'Add a device for bob'));
  const text = p.dom.byId.get('aw-pair').textContent;
  assert.match(text, /Android app/);
  assert.match(text, /web browser.*username and password/);
});

// --- trusted networks and http:// to a private address (docs/plans/trusted-networks.md) -----

// The vectors shared with the gateway (core/src/privnet.rs) and the plugin's Perl check.
const VECTORS = JSON.parse(readFileSync(new URL('../../../web/app/tests/fixtures/private-networks.json', import.meta.url), 'utf8'));

test('private addresses follow the shared vectors', () => {
  for (const a of VECTORS.addresses.private) {
    assert.ok(t.isPrivateAddress(a), a);
    if (a.includes(':')) assert.ok(t.isPrivateAddress(`[${a}]`), `[${a}]`);
  }
  for (const a of VECTORS.addresses.not_private) assert.ok(!t.isPrivateAddress(a), a);
  // Names are never private (they could resolve anywhere), nor is anything that isn't a literal.
  for (const a of ['localhost', 'lms.example.lan', 'music.example.com', '10.8.0.1.example.com', '010.8.0.1', '10.8.1', '0x0a.8.0.1',
    '167772161', '10.8.0.256', '10.8.0.1 ', ' 10.8.0.1', '10.8.0.1\n', 'fe80::1%eth0', 'fd00:::1', 'fd00::1::2', '1:2:3:4:5:6:7:8:9',
    'fd00:12345::1', 'fd00::g', '[fd00::1', 'fd00::1]', '١٠.8.0.1', '', null, undefined, 10, ['10.8.0.1'], { toString: () => '10.8.0.1' }]) {
    assert.ok(!t.isPrivateAddress(a), String(a));
  }
  // The edges of each range.
  for (const [inside, outside] of [['10.255.255.255', '11.0.0.0'], ['172.31.255.255', '172.32.0.0'], ['192.168.255.255', '192.169.0.0'],
    ['169.254.255.255', '169.255.0.0'], ['100.127.255.255', '100.128.0.0'], ['100.64.0.0', '100.63.255.255'], ['fdff:ffff::1', 'fbff:ffff::1'],
    ['febf:ffff::1', 'fec0::']]) {
    assert.ok(t.isPrivateAddress(inside), inside);
    assert.ok(!t.isPrivateAddress(outside), outside);
  }
  assert.ok(t.isPrivateAddress('fd00::10.0.0.1'), 'an IPv4 tail on a private IPv6 address');
  assert.ok(!t.isPrivateAddress('::ffff:192.168.0.1'), 'an IPv4-mapped address is not taken as IPv4');
});

test('an http:// Public URL with a literal private address: its origin, else null', () => {
  const ok = {
    'http://10.8.0.1:9443': 'http://10.8.0.1:9443',
    ' HTTP://10.8.0.1:9443/ ': 'http://10.8.0.1:9443',
    'http://10.8.0.1': 'http://10.8.0.1',
    'http://10.8.0.1:80': 'http://10.8.0.1',
    'http://100.101.102.103:09443': 'http://100.101.102.103:9443',
    'http://[FD00::1]:9443': 'http://[fd00::1]:9443',
    'http://[fe80:0:0:0::1]': 'http://[fe80::1]',
    'http://[fd00:0:1:0:0:1:0:0]:1': 'http://[fd00:0:1::1:0:0]:1',
  };
  for (const [u, want] of Object.entries(ok)) assert.equal(t.privateHttpOrigin(u), want, u);
  for (const a of VECTORS.addresses.private) {
    assert.ok(t.privateHttpOrigin(`http://${a.includes(':') ? `[${a}]` : a}:9443`), a);
  }
  for (const a of VECTORS.addresses.not_private) {
    assert.equal(t.privateHttpOrigin(`http://${a.includes(':') ? `[${a}]` : a}:9443`), null, a);
  }
  for (const bad of ['https://10.8.0.1:9443', 'http://music.example.com', 'http://lms.example.lan:9443', 'http://10.8.0.1.example.com',
    'http://8.8.8.8', 'http://127.0.0.1:9443', 'http://10.8.0.1:9443/lms', 'http://10.8.0.1:9443?x', 'http://10.8.0.1:9443#x',
    'http://user@10.8.0.1', 'http://10.8.0.1:0', 'http://10.8.0.1:70000', 'http://010.8.0.1', 'http://10.8.1', 'http://0x0a.8.0.1',
    'http://167772161', 'http://[::ffff:10.0.0.1]', 'http://[fe80::1%25eth0]', 'http://fd00::1', 'http://10.8.0.1\n:9443',
    'ftp://10.8.0.1', 'http://', 'javascript:alert(1)//10.8.0.1', '', null, undefined, 42, `http://10.8.0.1:9443${'/'.repeat(300)}`]) {
    assert.equal(t.privateHttpOrigin(bad), null, String(bad));
  }
});

test('pairing links: the encoder’s for https://, and for http:// only to a private address', () => {
  // An encoder that refuses every http:// address (as qr.js did before trusted networks), so
  // this does not depend on the generated copy of the web app's encoder.
  const qr = {
    PAIRING_SCHEME: 'lyrion-anywhere',
    normalisePairingCode: (c) => (typeof c === 'string' && /^[A-Z2-9]{10}$/.test(c) ? c : null),
    pairingLink: (u, c) => (u === 'https://music.example.com' && c === 'ABCDEFGHJK' ? 'from-the-encoder' : null),
    groupPairingCode: (c) => `${c.slice(0, 5)}-${c.slice(5)}`,
  };
  assert.equal(t.pairingLinkFor('https://music.example.com', 'ABCDEFGHJK', qr), 'from-the-encoder', 'https: the encoder decides');
  assert.equal(t.pairingLinkFor('http://10.8.0.1:9443', 'ABCDEFGHJK', qr), 'lyrion-anywhere://pair?server=http%3A%2F%2F10.8.0.1%3A9443&code=ABCDEFGHJK');
  assert.equal(t.pairingLinkFor('http://[FD00::1]:9443/', 'ABCDEFGHJK', qr), 'lyrion-anywhere://pair?server=http%3A%2F%2F%5Bfd00%3A%3A1%5D%3A9443&code=ABCDEFGHJK');
  for (const url of ['http://music.example.com', 'http://8.8.8.8:9443', 'http://10.8.0.1:9443/lms', '', null]) {
    assert.equal(t.pairingLinkFor(url, 'ABCDEFGHJK', qr), null, String(url));
  }
  for (const code of ['', 'abc', 'ABCD&server=https://evil.example', null]) {
    assert.equal(t.pairingLinkFor('http://10.8.0.1:9443', code, qr), null, String(code));
  }
  const view = t.pairingView({ code: 'ABCDEFGHJK', expires_in: 600 }, 'http://10.8.0.1:9443', qr);
  assert.equal(view.link, 'lyrion-anywhere://pair?server=http%3A%2F%2F10.8.0.1%3A9443&code=ABCDEFGHJK', 'the panel uses it');
  assert.equal(t.pairingView({ code: 'ABCDEFGHJK', expires_in: 600 }, 'http://music.example.com', qr).link, null, 'never for an http:// name');
});

test('the Trusted networks warning and the note about the Listen address', () => {
  for (const v of ['10.8.0.0/24', ' x ', 'evil']) assert.ok(t.hasTrustedNetworks(v), v);
  for (const v of ['', '   ', null, undefined]) assert.ok(!t.hasTrustedNetworks(v), String(v));
  assert.equal(t.trustedListenText('', '127.0.0.1:9443'), null, 'no networks: nothing to say');
  assert.equal(t.trustedListenText('10.8.0.0/24', '10.8.0.1:9443'), null, 'the VPN address: fine');
  assert.equal(t.trustedListenText('10.8.0.0/24', '[fd00::1]:9443'), null, 'an IPv6 VPN address: fine');
  assert.match(t.trustedListenText('10.8.0.0/24', '127.0.0.1:9443'), /listens on 127\.0\.0\.1:9443, which devices on your VPN cannot reach/);
  assert.match(t.trustedListenText('10.8.0.0/24', ''), /127\.0\.0\.1:9443/, 'empty: the default, loopback');
  assert.match(t.trustedListenText('10.8.0.0/24', '[::1]:9443'), /cannot reach/);
  assert.match(t.trustedListenText('10.8.0.0/24', '0.0.0.0:9443'), /every network \(0\.0\.0\.0:9443\)/);
  assert.match(t.trustedListenText('10.8.0.0/24', '[::]:9443'), /every network/);
  assert.ok(!/[^\x20-\x7e’]/.test(t.trustedListenText('10.8.0.0/24', '127.0.0.1:9443')), 'plain text');
});

test('the warnings show as the admin types', () => {
  const dom = fakeDom();
  const node = (id, value) => {
    const n = dom.document.createElement('input');
    n.value = value;
    dom.byId.set(id, n);
    return n;
  };
  const nets = node('pref_trusted_networks', '');
  const listen = node('pref_listen', '127.0.0.1:9443');
  const warning = node('aw-trusted-warning', '');
  const note = node('aw-trusted-listen', '');
  const url = node('pref_public_url', '');
  const httpWarning = node('aw-http-warning', '');
  warning.setAttribute('hidden', '');
  note.setAttribute('hidden', '');
  const fetch = async () => ({ json: async () => ({ result: {} }) });
  const ctx = { document: dom.document, fetch, URL, setInterval: () => 0, setTimeout: () => 0, clearTimeout: () => {}, clearInterval: () => {}, Map, WeakMap, Set, Promise, JSON, Date, Math, Object, String, Error };
  ctx.globalThis = ctx;
  vm.runInNewContext(qrSrc, ctx);
  vm.runInNewContext(src, ctx);
  dom.ready();
  const type = (input, v) => { input.value = v; for (const fn of input.listeners.input || []) fn({ target: input }); };
  assert.ok(warning.hasAttribute('hidden') && note.hasAttribute('hidden'), 'empty: no warning');
  type(nets, '10.8.0.0/24');
  assert.ok(!warning.hasAttribute('hidden'), 'typing a network shows the warning');
  assert.ok(!note.hasAttribute('hidden'), 'and, with the listen address on loopback, the note');
  assert.match(note.textContent, /cannot reach/);
  type(listen, '10.8.0.1:9443');
  assert.ok(note.hasAttribute('hidden'), 'the VPN address: the note goes');
  assert.ok(!warning.hasAttribute('hidden'), 'the warning stays');
  type(listen, '0.0.0.0:9443');
  assert.match(note.textContent, /every network/);
  type(nets, '  ');
  assert.ok(warning.hasAttribute('hidden') && note.hasAttribute('hidden'), 'cleared: both go');
  type(url, 'http://10.8.0.1:9443');
  assert.ok(!httpWarning.hasAttribute('hidden'), 'the http:// warning still works');
});
