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
  assert.equal(t.formatTime(0), '–');
  assert.equal(t.formatUptime(0), '–');
  assert.equal(t.formatUptime(125), '2 min');
  assert.equal(t.formatUptime(3725), '1 h 2 min');
});

// --- the whole page, rendered into a minimal DOM with hostile data -----------------------

/** A tiny DOM: text is only ever text, and any attempt to parse HTML throws. */
function fakeDom() {
  const created = [];
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
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    append(...c) { this.children.push(...c); }
    replaceChildren(...c) { this._text = ''; this.children = c; }
    focus() {}
  }
  const byId = new Map();
  for (const id of ['anywhere-app', 'aw-status', 'aw-users', 'aw-devices', 'aw-audit', 'aw-message', 'aw-create-btn', 'aw-new-user', 'aw-new-password', 'aw-new-role', 'aw-session-days', 'aw-session-btn', 'aw-plugins', 'aw-plugins-btn']) {
    byId.set(id, new Node('div'));
  }
  const docListeners = {};
  const document = {
    createElement: (tag) => new Node(tag),
    getElementById: (id) => byId.get(id),
    addEventListener: (type, fn) => { docListeners[type] = fn; },
    write() { throw new Error('document.write used'); },
  };
  return { document, byId, created, ready: () => docListeners.DOMContentLoaded() };
}

const HOSTILE = '<img src=x onerror=document.title="PWNED">';

test('renders remote-controlled names as text, never as markup', async () => {
  const dom = fakeDom();
  const replies = {
    status: { running: 1, enabled: 1, pid: 42, uptime: 60, listen: '127.0.0.1:9443', binary_ok: 1, binary: '/x', data_dir: '/d' },
    players: { players_loop: [{ playerid: 'aa:bb:cc:dd:ee:01', name: HOSTILE }] },
    users: { data: { users: [{ username: 'bob', role: 'listener', totp_enabled: 0, disabled: 0, downloads: true, download_formats: ['original', 'opus', HOSTILE] }] } },
    grants: { data: { players: ['aa:bb:cc:dd:ee:01'] } },
    devices: { data: { devices: [{ id: 1, name: HOSTILE, username: 'bob', mac: '02:4c:41:00:00:09', last_seen: 1 }] } },
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
  for (const c of sent.filter((c) => c[0] === 'anywhere' && c[1] !== 'status')) {
    assert.equal(c[c.length - 1], 'csrf:' + TOKEN, `token sent with ${c[1]}`);
  }
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
