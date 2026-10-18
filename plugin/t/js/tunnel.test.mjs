// The settings page's Cloudflare Tunnel section: the state in words, the controls for each
// state, input checks that match Admin.pm, the commands each button sends (always with the
// page's token), secrets emptied once sent, the temporary tunnel's link, QR code and
// countdown, and the tunnel's address used for pairing codes. Nothing from the gateway is
// ever treated as markup.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const dir = new URL('../../Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/', import.meta.url);
const src = readFileSync(new URL('anywhere.js', dir), 'utf8');
const qrSrc = readFileSync(new URL('qr.js', dir), 'utf8');
const pluginDir = fileURLToPath(new URL('../../', import.meta.url));

const t = (() => {
  const ctx = { __anywhereTest: {}, URL, atob, JSON };
  ctx.globalThis = ctx;
  vm.runInNewContext(src, ctx);
  return ctx.__anywhereTest;
})();

const b64 = (s) => Buffer.from(s, 'utf8').toString('base64');
const tokenOf = (o) => b64(JSON.stringify(o));
const TUNNEL_TOKEN = tokenOf({ a: '0123456789abcdef0123456789abcdef', t: '01234567-89ab-cdef-0123-456789abcdef', s: b64('z'.repeat(32)) });
const API_TOKEN = 'Abc_def-0123456789xyzABCDEFGHIJKLMNOPQRS';

// --- input checks: the same answers as Admin.pm ---------------------------------------------

const HOSTNAMES_OK = ['music.example.com', 'a.bc', 'MUSIC.Example.Com', 'x-1.my-home.example.co.uk', 'a1.b2', 'a'.repeat(63) + '.example.com', 'evil-trycloudflare.com'];
const HOSTNAMES_BAD = [
  'a.b', '', 'localhost', 'musicexamplecom', Array(4).fill('a'.repeat(63)).join('.'), 'a'.repeat(64) + '.example.com',
  '-music.example.com', 'music-.example.com', 'music..example.com', '.music.example.com', 'music.example.com.',
  'my_music.example.com', 'music.example.com:443', 'music.example.com/lms', 'https://music.example.com', '10.0.0.10',
  '[::1]', 'fe80::1', 'music.example.123', 'random-words.trycloudflare.com', 'trycloudflare.com', '0123.cfargotunnel.com',
  'CFArgoTunnel.com', 'music example.com', 'music.example.com\r\nX-Evil: 1', 'müsic.example.com', '../music.example.com',
  'user@music.example.com', 'music.example.com?x=1',
];
const TOKENS_OK = [TUNNEL_TOKEN, TUNNEL_TOKEN.replace(/=+$/, ''), tokenOf({ a: 'x', t: 'y', s: '?'.repeat(40) }), tokenOf({ a: 'x', t: 'y', s: '?'.repeat(40) }).replace(/\+/g, '-').replace(/\//g, '_')];
const TOKENS_BAD = [
  'eyJhIjoiYSJ9', '', 'A'.repeat(4097), TUNNEL_TOKEN.slice(0, 70) + '!!!!', '../'.repeat(30), TUNNEL_TOKEN + '===',
  b64('x'.repeat(80)), b64('["a","t","s","' + 'x'.repeat(60) + '"]'),
  tokenOf({ t: 'y'.repeat(32), s: 'z'.repeat(44) }), tokenOf({ a: 'x'.repeat(32), s: 'z'.repeat(44) }), tokenOf({ a: 'x'.repeat(32), t: 'y'.repeat(32) }),
  tokenOf({ a: 'x'.repeat(32), t: 'y'.repeat(32), s: '' }), tokenOf({ a: 'x'.repeat(32), t: 'y'.repeat(32), s: { x: 1 } }),
  TUNNEL_TOKEN + '\r\nX-Evil: 1',
];
const API_OK = [API_TOKEN, 'a'.repeat(20), 'a'.repeat(128)];
const API_BAD = ['a'.repeat(19), 'a'.repeat(129), 'a'.repeat(30) + '+/', 'a'.repeat(30) + '.b', 'Bearer ' + 'a'.repeat(40), '', 'a'.repeat(40) + '\r\nX: y'];

test('hostnames: the rules of Admin.pm, lower-cased', () => {
  for (const h of HOSTNAMES_OK) assert.equal(t.validTunnelHostname(h), h.toLowerCase(), h);
  for (const h of HOSTNAMES_BAD) assert.equal(t.validTunnelHostname(h), null, JSON.stringify(h));
  assert.equal(t.validTunnelHostname('  Music.Example.com '), 'music.example.com', 'trimmed');
  for (const v of [null, undefined, 42, ['music.example.com']]) assert.equal(t.validTunnelHostname(v), null, String(v));
});

test('tunnel tokens: base64 JSON with a, t and s; spaces and line breaks from copying removed', () => {
  for (const tok of TOKENS_OK) assert.equal(t.validTunnelToken(tok), tok, tok.slice(0, 20));
  for (const tok of TOKENS_BAD) assert.equal(t.validTunnelToken(tok), null, JSON.stringify(tok.slice(0, 40)));
  const wrapped = TUNNEL_TOKEN.slice(0, 40) + '\n  ' + TUNNEL_TOKEN.slice(40) + '\n';
  assert.equal(t.validTunnelToken(wrapped), TUNNEL_TOKEN, 'a wrapped paste is joined up');
  for (const v of [null, undefined, 42, [TUNNEL_TOKEN]]) assert.equal(t.validTunnelToken(v), null, String(v));
});

test('API tokens: 20 to 128 characters of A-Z a-z 0-9 _ -', () => {
  for (const a of API_OK) assert.equal(t.validApiToken(a), a);
  for (const a of API_BAD) assert.equal(t.validApiToken(a), null, a);
  assert.equal(t.validApiToken(` ${API_TOKEN} `), API_TOKEN, 'trimmed');
});

test('the page and Admin.pm agree on every case', (ctx) => {
  const cases = [
    ...[...HOSTNAMES_OK, ...HOSTNAMES_BAD].map((v) => ['valid_hostname', v, t.validTunnelHostname(v) !== null]),
    ...[...TOKENS_OK, ...TOKENS_BAD].map((v) => ['valid_tunnel_token', v, t.validTunnelToken(v) !== null]),
    ...[...API_OK, ...API_BAD].map((v) => ['valid_api_token', v, t.validApiToken(v) !== null]),
  ].filter(([, v]) => v === v.trim()); // the page trims, Admin.pm refuses: not the same question
  const perl = `
    use strict; use warnings; use JSON::PP; use Plugins::LyrionAnywhere::Admin;
    local $/; my $cases = JSON::PP->new->utf8->decode(<STDIN>);
    print join('', map { no strict 'refs'; (&{"Plugins::LyrionAnywhere::Admin::$_->[0]"}($_->[1]) ? 1 : 0) } @$cases);
  `;
  const r = spawnSync('perl', ['-It/stubs', '-I.', '-e', perl], { cwd: pluginDir, input: JSON.stringify(cases.map(([f, v]) => [f, v])), encoding: 'utf8' });
  if (r.error) return ctx.skip('perl is not available');
  assert.equal(r.status, 0, r.stderr);
  const got = r.stdout.split('');
  assert.equal(got.length, cases.length);
  cases.forEach(([f, v, js], i) => assert.equal(got[i] === '1', js, `${f}(${JSON.stringify(v.slice(0, 50))}): Perl ${got[i]}, page ${js}`));
});

// --- the commands -------------------------------------------------------------------------

test('each button sends its command', () => {
  for (const [kind, op] of [['install', 'tunnel_install'], ['update', 'tunnel_update'], ['start', 'tunnel_start'], ['stop', 'tunnel_stop'], ['quick', 'tunnel_quick'], ['remove', 'tunnel_remove'], ['log', 'tunnel_log']]) {
    const r = t.tunnelRequest(kind);
    assert.equal(r.op, op);
    assert.deepEqual({ ...r.args }, {});
  }
  const reg = t.tunnelRequest('register', { token: TUNNEL_TOKEN, hostname: ' Music.Example.com ' });
  assert.deepEqual({ op: reg.op, args: { ...reg.args } }, { op: 'tunnel_register', args: { token: TUNNEL_TOKEN, hostname: 'music.example.com' } });
  const tok = 'ab'.repeat(16);
  assert.deepEqual(Array.from(t.command(reg.op, reg.args, tok)), ['anywhere', 'tunnel_register', 'token:' + TUNNEL_TOKEN, 'hostname:music.example.com', 'csrf:' + tok]);
  for (const replace of [true, false]) {
    const c = t.tunnelRequest('create', { api_token: API_TOKEN, hostname: 'music.example.com', replace_dns: replace });
    assert.deepEqual(Array.from(t.command(c.op, c.args, tok)),
      ['anywhere', 'tunnel_create', 'api_token:' + API_TOKEN, 'hostname:music.example.com', `replace_dns:${replace ? 1 : 0}`, 'csrf:' + tok], 'replace_dns 0 is sent, not dropped');
  }
  assert.deepEqual(Array.from(t.command('tunnel_status', {}, tok)), ['anywhere', 'tunnel_status', 'csrf:' + tok], 'status carries the token too');
  // Bad input: an error to show, nothing to send, and the value not repeated.
  for (const [kind, fields, why] of [
    ['register', { token: 'x'.repeat(70), hostname: 'music.example.com' }, /tunnel token/],
    ['register', { token: TUNNEL_TOKEN, hostname: 'music.example.com:443' }, /hostname/],
    ['create', { api_token: 'short', hostname: 'music.example.com' }, /API token/],
    ['create', { api_token: API_TOKEN, hostname: '10.0.0.10' }, /hostname/],
    ['nonsense', {}, /Unknown/],
  ]) {
    const r = t.tunnelRequest(kind, fields);
    assert.equal(r.op, undefined, kind);
    assert.match(r.error, why);
    if (fields.api_token) assert.ok(!r.error.includes(fields.api_token));
  }
});

test('secret fields are emptied', () => {
  const a = { value: TUNNEL_TOKEN };
  const b = { value: API_TOKEN };
  t.clearFields(a, b, null, undefined);
  assert.equal(a.value, '');
  assert.equal(b.value, '');
});

// --- the state in words, and the controls for it ------------------------------------------

const BASE = {
  supported: true, pinned_version: '2026.10.1', installed_version: '2026.10.1', update_available: false, mode: 'none', state: 'installed',
  hostname: null, public_url: null, connections: 0, uptime: null, quick_ends_in: null, progress: null, step: null, error: null, tunnel_listen: '127.0.0.1:9444', log: [],
};
const S = (o) => ({ ...BASE, ...o });
const HOSTILE_TEXT = '<img src=x onerror=alert(1)>';

test('every state in words', () => {
  const words = {
    unsupported: [S({ supported: false, state: 'unsupported', installed_version: null }), /not available on this server.*x86_64, aarch64 or armhf/],
    not_installed: [S({ state: 'not_installed', installed_version: null }), /^Not installed\. Nothing is downloaded or run until you press Install/],
    installing: [S({ state: 'installing', installed_version: null, progress: { bytes: 12345678, total: 38000000 } }), /^Installing cloudflared: 12\.3 of 38\.0 MB downloaded/],
    installed: [S({}), /^Installed\. No tunnel is registered yet/],
    creating: [S({ state: 'creating', step: 'dns' }), /^Setting up the tunnel in your Cloudflare account: adding the DNS record/],
    starting: [S({ state: 'starting', mode: 'registered', hostname: 'music.example.com' }), /^Starting: connecting to Cloudflare/],
    connected: [S({ state: 'connected', mode: 'registered', connections: 4 }), /^Connected to Cloudflare \(4 connections\)\.$/],
    reconnecting: [S({ state: 'reconnecting', mode: 'registered' }), /not connected to Cloudflare: trying again.*on port 7844 \(UDP or TCP\)\.$/],
    stopped: [S({ state: 'stopped', mode: 'registered', hostname: 'music.example.com' }), /^Stopped\. The tunnel for music\.example\.com stays registered/],
    failed: [S({ state: 'failed', error: 'dns: a DNS record for music.example.com already exists' }), /^Failed: dns: a DNS record for music\.example\.com already exists$/],
  };
  for (const [state, [s, want]] of Object.entries(words)) assert.match(t.tunnelStateText(s), want, state);
  assert.equal(Object.keys(words).length, 10, 'all ten states of the contract');
  // Details within states.
  assert.match(t.tunnelStateText(S({ state: 'installing', progress: { bytes: 2500000, total: null } })), /2\.5 MB downloaded/);
  assert.match(t.tunnelStateText(S({ state: 'installing', progress: null })), /starting the download/);
  assert.match(t.tunnelStateText(S({ state: 'connected', connections: 1 })), /\(1 connection\)/);
  assert.match(t.tunnelStateText(S({ state: 'installed', mode: 'registered', hostname: 'music.example.com' })), /registered for music\.example\.com, but not running/);
  for (const [step, words] of [['zone', 'finding your domain'], ['create', 'creating the tunnel'], ['configure', 'pointing the tunnel at the gateway'], ['token', 'fetching the tunnel token'], ['__proto__', 'working'], [null, 'working']]) {
    assert.ok(t.tunnelStateText(S({ state: 'creating', step })).includes(words), String(step));
  }
  assert.match(t.tunnelStateText(S({ state: 'connected', mode: 'quick', connections: 2 })), /^Temporary tunnel\. Connected/);
  assert.match(t.tunnelStateText(S({ state: 'failed', error: null })), /no reason/);
  // Unsupported because the gateway could not open the tunnel's own listener: its sentence.
  const busy = "The tunnel's own listener could not be opened on 127.0.0.1:9444 (Address in use), so the tunnel cannot run.";
  assert.equal(t.tunnelStateText(S({ supported: false, state: 'unsupported', tunnel_listen: null, error: busy })), `Cloudflare Tunnel is not available on this server. ${busy}`);
  assert.doesNotMatch(t.tunnelStateText(S({ state: 'reconnecting' })), /443/);
  assert.match(t.tunnelStateText(S({ state: 'something_new' })), /Unknown tunnel state: something_new/);
  assert.match(t.tunnelStateText(null), /no tunnel status/);
});

test('the buttons and forms for each state', () => {
  const on = (s) => Object.entries(t.tunnelControls(s)).filter(([, v]) => v).map(([k]) => k).sort().join(' ');
  assert.equal(on(S({ supported: false, state: 'unsupported', installed_version: null })), '');
  assert.equal(on(S({ state: 'not_installed', installed_version: null })), 'install quick');
  assert.equal(on(S({ state: 'installing', installed_version: null })), '', 'nothing while downloading');
  assert.equal(on(S({})), 'forms quick remove', 'installed, nothing registered: the forms');
  assert.equal(on(S({ update_available: true, installed_version: '2026.9.0' })), 'forms quick remove update');
  assert.equal(on(S({ state: 'creating' })), '', 'nothing while creating');
  assert.equal(on(S({ state: 'connected', mode: 'registered', hostname: 'music.example.com' })), 'remove stop');
  assert.equal(on(S({ state: 'connected', mode: 'registered', hostname: 'music.example.com', update_available: true })), 'remove stop update');
  assert.equal(on(S({ state: 'reconnecting', mode: 'registered', hostname: 'music.example.com' })), 'remove stop');
  assert.equal(on(S({ state: 'stopped', mode: 'registered', hostname: 'music.example.com' })), 'quick remove start');
  assert.equal(on(S({ state: 'installed', mode: 'registered', hostname: 'music.example.com' })), 'quick remove start');
  assert.equal(on(S({ state: 'connected', mode: 'quick', hostname: 'a-b-c.trycloudflare.com' })), 'forms quickStop remove', 'a temporary tunnel: its own Stop');
  assert.equal(on(S({ state: 'failed', installed_version: null })), 'install quick', 'a failed download: Install again');
  assert.equal(on(S({ state: 'failed' })), 'forms quick remove', 'a failed create: the forms again');
  assert.equal(on(null), '');
});

test('versions, countdown, log and addresses', () => {
  assert.equal(t.tunnelVersionText(S({})), '2026.10.1');
  assert.equal(t.tunnelVersionText(S({ installed_version: '2026.9.0' })), '2026.9.0 (this release pins 2026.10.1)');
  assert.equal(t.tunnelVersionText(S({ installed_version: null })), 'not installed (this release pins 2026.10.1)');
  assert.equal(t.quickCountdownText(3600), 'Stops in 60:00.');
  assert.equal(t.quickCountdownText(65), 'Stops in 1:05.');
  assert.equal(t.quickCountdownText(0), 'Stopping…');
  assert.equal(t.quickCountdownText('x'), 'Stopping…');
  assert.equal(t.tunnelLogText(['one', '<b>two</b>', 3, null, 'bell\u0007\u001b[31m']), 'one\n<b>two</b>\nbell  [31m');
  assert.equal(t.tunnelLogText('not a list'), '');
  assert.equal(t.formatMB(0), '0.0');
  assert.equal(t.formatMB(1e6), '1.0');
  // Only plain https:// addresses become links or QR codes.
  assert.equal(t.tunnelUrl('https://music.example.com'), 'https://music.example.com');
  assert.equal(t.tunnelUrl('https://a-b-c.trycloudflare.com/'), 'https://a-b-c.trycloudflare.com');
  for (const bad of ['javascript:alert(1)', 'http://music.example.com', 'https://music.example.com/x', 'https://evil.example"onclick=x', null, 42]) {
    assert.equal(t.tunnelUrl(bad), null, String(bad));
  }
});

test('pairing codes use the tunnel address while there is one', () => {
  assert.equal(t.pairingPublicUrl(S({ public_url: 'https://a-b-c.trycloudflare.com' }), 'https://music.example.com'), 'https://a-b-c.trycloudflare.com');
  assert.equal(t.pairingPublicUrl(S({ public_url: null }), 'https://music.example.com'), 'https://music.example.com');
  assert.equal(t.pairingPublicUrl(null, 'https://music.example.com'), 'https://music.example.com');
  assert.equal(t.pairingPublicUrl(S({ public_url: 'javascript:alert(1)' }), ''), '');
  // The override notes: only while a tunnel runs, not merely registered.
  for (const state of ['starting', 'connected', 'reconnecting']) {
    const o = t.tunnelOverrideText(S({ state, mode: 'registered', public_url: 'https://music.example.com' }));
    assert.equal(o.url, 'Overridden by the Cloudflare Tunnel while it runs: the gateway uses https://music.example.com.', state);
    assert.equal(o.header, 'Overridden by the Cloudflare Tunnel while it runs: the gateway reads CF-Connecting-IP, but only on requests that arrive through the tunnel’s own listener.', state);
  }
  for (const state of ['installed', 'stopped', 'failed', 'creating', 'installing']) {
    assert.deepEqual({ ...t.tunnelOverrideText(S({ state, mode: 'registered', public_url: 'https://music.example.com' })) }, { url: null, header: null }, state);
  }
  assert.deepEqual({ ...t.tunnelOverrideText(S({ state: 'connected' })) }, { url: null, header: null });
  assert.deepEqual({ ...t.tunnelOverrideText(null) }, { url: null, header: null });
});

test('the token form names the tunnel listener as the service to set in Cloudflare', () => {
  assert.equal(t.tunnelServiceText(S({})), 'In Cloudflare, give the tunnel the public hostname you want and set its service to http://127.0.0.1:9444.');
  assert.equal(t.tunnelServiceText(S({ tunnel_listen: '[::1]:9500' })), 'In Cloudflare, give the tunnel the public hostname you want and set its service to http://[::1]:9500.');
  // No listener, or anything that isn't a plain loopback address: the fallback, never the value.
  for (const bad of [null, '', '10.0.0.10:9444', '0.0.0.0:9444', '127.0.0.1:9444/<b>x</b>', 'javascript:alert(1)', 42, HOSTILE_TEXT]) {
    const text = t.tunnelServiceText(S({ tunnel_listen: bad }));
    assert.match(text, /^In Cloudflare, give the tunnel the public hostname you want and set its service to the gateway’s tunnel listener \(http:\/\/127\.0\.0\.1:9444 unless/, String(bad));
    assert.ok(typeof bad !== 'string' || bad === '' || !text.includes(bad), String(bad));
  }
  assert.equal(t.tunnelServiceText(null), t.tunnelServiceText(S({ tunnel_listen: null })));
  assert.equal(t.tunnelListen(S({ tunnel_listen: '127.0.0.1:9444' })), '127.0.0.1:9444');
  assert.equal(t.tunnelListen(null), null);
});

// --- the section on the page --------------------------------------------------------------

/** A tiny DOM (as in settings.test.mjs): every id exists, text is only text, HTML parsing throws. */
function fakePage(replies) {
  const created = [];
  class Node {
    constructor(tag) {
      this.tagName = tag.toUpperCase();
      this.children = [];
      this.attrs = {};
      this.listeners = {};
      this._text = '';
      this.value = '';
      this.checked = false;
      this.disabled = false;
      this.className = '';
      this.classList = { add() {}, remove() {} };
      created.push(this);
    }
    set textContent(v) { this._text = String(v); this.children = []; }
    get textContent() { return this._text + this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join(''); }
    set innerHTML(_) { throw new Error('innerHTML used'); }
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
    get isConnected() { return true; }
    focus() {}
  }
  const byId = new Map();
  const node = (id) => {
    if (!byId.has(id)) byId.set(id, new Node('div'));
    return byId.get(id);
  };
  const TOKEN = 'ab'.repeat(16);
  node('anywhere-app').setAttribute('data-csrf', TOKEN);
  node('anywhere-app').setAttribute('data-public-url', 'https://music.example.com');
  for (const id of ['aw-pair', 'aw-tunnel-remove-note', 'aw-tunnel-token-form', 'aw-tunnel-create-form', 'aw-tunnel-quick-box']) node(id).setAttribute('hidden', '');
  const listeners = {};
  const document = {
    createElement: (tag) => new Node(tag),
    createElementNS: (_ns, tag) => new Node(tag),
    getElementById: node,
    activeElement: null,
    body: null,
    hidden: false,
    querySelectorAll: () => created.filter((e) => 'data-aw-key' in e.attrs),
    addEventListener: (type, fn) => { listeners[type] = fn; },
  };
  const sent = [];
  let hold = null; // when set, tunnel commands wait for it
  const fetch = async (_url, opts) => {
    const cmd = JSON.parse(opts.body).params[1];
    sent.push(cmd);
    if (hold && cmd[1] !== 'tunnel_status' && String(cmd[1]).startsWith('tunnel_')) await hold;
    const key = cmd[0] === 'players' ? 'players' : cmd[1];
    const r = replies[key];
    return { json: async () => ({ result: (typeof r === 'function' ? r(cmd) : r) || {} }) };
  };
  const copied = [];
  const navigator = { clipboard: { writeText: async (s) => { copied.push(s); } } };
  const intervals = [];
  const ctx = {
    document, fetch, navigator, URL, atob, Map, WeakMap, Set, Promise, JSON, Date, Math, Object, String, Number, Error, Array,
    setInterval: (fn, ms) => { intervals.push([fn, ms]); return intervals.length; }, clearInterval: () => {}, setTimeout: () => 0, clearTimeout: () => {},
  };
  ctx.globalThis = ctx;
  vm.runInNewContext(qrSrc, ctx);
  vm.runInNewContext(src, ctx);
  const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
  const click = async (id) => { const b = typeof id === 'string' ? node(id) : id; for (const fn of b.listeners.click || []) fn({ target: b }); await settle(); };
  const isShown = (id) => !node(id).hasAttribute('hidden');
  return {
    node, sent, TOKEN, settle, click, isShown, copied, intervals, created, document, ctx, navigator,
    ready: async () => { listeners.DOMContentLoaded(); await settle(); },
    holdCommands: () => { let release; hold = new Promise((r) => { release = r; }); return () => { hold = null; release(); }; },
  };
}

const RUNNING = { running: 1, enabled: 1, pid: 1, uptime: 1, listen: '127.0.0.1:9443', binary_ok: 1 };
const GATEWAY = {
  status: RUNNING, players: { players_loop: [] }, users: { data: { users: [] } }, devices: { data: { devices: [] } },
  audit: { data: { audit: [] } }, settings: { data: {} }, plugins: { data: { plugins: [] } },
};
const HOSTILE = '<img src=x onerror=document.title="PWNED">';

test('the section shows the state, details and log as text, and polls every 3 seconds', async () => {
  let status = S({ state: 'connected', mode: 'registered', hostname: 'music.example.com', public_url: 'https://music.example.com', connections: 4, uptime: 3725, log: ['started', HOSTILE] });
  const p = fakePage({ ...GATEWAY, tunnel_status: () => ({ ok: 1, data: status }) });
  await p.ready();
  assert.equal(p.node('aw-tunnel-state').textContent, 'Connected to Cloudflare (4 connections).');
  const info = p.node('aw-tunnel-info').textContent;
  assert.ok(info.includes('https://music.example.com') && info.includes('1 h 2 min') && info.includes('2026.10.1'), info);
  const link = p.created.find((e) => e.tagName === 'A' && e.attrs.href === 'https://music.example.com');
  assert.ok(link, 'the public address is a link');
  assert.equal(p.node('aw-tunnel-log').textContent, 'started\n' + HOSTILE, 'log lines as text');
  assert.ok(p.isShown('aw-tunnel-log-box'));
  assert.equal(p.created.filter((e) => e.tagName === 'IMG' || e.tagName === 'SCRIPT').length, 0, 'nothing parsed as markup');
  assert.ok(p.isShown('aw-tunnel-stop') && !p.isShown('aw-tunnel-start') && !p.isShown('aw-tunnel-install'), 'Stop only');
  assert.ok(!p.isShown('aw-tunnel-token-form') && !p.isShown('aw-tunnel-create-form'), 'no forms while registered');
  // The notes under Public URL and Client address header.
  assert.ok(p.isShown('aw-tunnel-override-url') && p.isShown('aw-tunnel-override-header'));
  assert.match(p.node('aw-tunnel-override-url').textContent, /^Overridden by the Cloudflare Tunnel while it runs: the gateway uses https:\/\/music\.example\.com\.$/);
  // Polled every 3 seconds, with the page's token.
  const poll = p.intervals.find(([, ms]) => ms === 3000);
  assert.ok(poll, 'a 3-second poll');
  status = S({ state: 'reconnecting', mode: 'registered', hostname: 'music.example.com', public_url: 'https://music.example.com' });
  await poll[0]();
  await p.settle();
  assert.match(p.node('aw-tunnel-state').textContent, /trying again/);
  // Not while the tab is hidden.
  const n = p.sent.length;
  p.document.hidden = true;
  await poll[0]();
  assert.equal(p.sent.length, n, 'nothing asked while hidden');
  for (const c of p.sent.filter((c) => c[0] === 'anywhere')) assert.equal(c[c.length - 1], 'csrf:' + p.TOKEN, `token sent with ${c[1]}`);
  assert.ok(p.sent.some((c) => c[1] === 'tunnel_status'));
});

test('a stopped gateway: the tunnel is not asked about, and the page says why', async () => {
  const p = fakePage({ ...GATEWAY, status: { running: 0, enabled: 1 } });
  await p.ready();
  assert.ok(!p.sent.some((c) => c[1] === 'tunnel_status'));
  assert.match(p.node('aw-tunnel-state').textContent, /gateway is not running/);
  for (const id of ['aw-tunnel-install', 'aw-tunnel-start', 'aw-tunnel-stop']) assert.ok(!p.isShown(id), id);
});

test('registering: validated, sent with the token, the token field emptied whatever the answer', async () => {
  let answer = { ok: 1, data: {} };
  const p = fakePage({ ...GATEWAY, tunnel_status: { ok: 1, data: S({}) }, tunnel_register: () => answer });
  await p.ready();
  assert.ok(p.isShown('aw-tunnel-token-form') && p.isShown('aw-tunnel-create-form'), 'the forms, when installed and nothing is registered');
  assert.equal(p.node('aw-tunnel-service-note').textContent, 'In Cloudflare, give the tunnel the public hostname you want and set its service to http://127.0.0.1:9444.');
  // Bad input: not sent, said why, the field kept so it can be corrected.
  p.node('aw-tunnel-token').value = 'not a token';
  p.node('aw-tunnel-token-host').value = 'music.example.com';
  await p.click('aw-tunnel-register-btn');
  assert.ok(!p.sent.some((c) => c[1] === 'tunnel_register'), 'a bad token is not sent');
  assert.match(p.node('aw-message').textContent, /not a tunnel token/);
  // Good input.
  p.node('aw-tunnel-token').value = TUNNEL_TOKEN;
  p.node('aw-tunnel-token-host').value = 'Music.Example.com';
  await p.click('aw-tunnel-register-btn');
  assert.deepEqual(p.sent.find((c) => c[1] === 'tunnel_register'), ['anywhere', 'tunnel_register', 'token:' + TUNNEL_TOKEN, 'hostname:music.example.com', 'csrf:' + p.TOKEN]);
  assert.equal(p.node('aw-tunnel-token').value, '', 'token emptied');
  assert.equal(p.node('aw-tunnel-token-host').value, 'Music.Example.com', 'the hostname kept');
  assert.ok(!p.node('aw-message').textContent.includes(TUNNEL_TOKEN), 'the token is not shown');
  // Refused by the gateway: still emptied, the reason shown.
  answer = { error: 'refused by the gateway: the tunnel token was refused' };
  p.node('aw-tunnel-token').value = TUNNEL_TOKEN;
  await p.click('aw-tunnel-register-btn');
  assert.equal(p.node('aw-tunnel-token').value, '');
  assert.equal(p.node('aw-message').textContent, 'refused by the gateway: the tunnel token was refused');
});

test('creating: the API token sent once and emptied, replace_dns from the box', async () => {
  const p = fakePage({ ...GATEWAY, tunnel_status: { ok: 1, data: S({}) }, tunnel_create: { error: 'refused by the gateway: zone: no zone on this account contains music.example.com' } });
  await p.ready();
  p.node('aw-tunnel-api-token').value = API_TOKEN;
  p.node('aw-tunnel-create-host').value = 'music.example.com';
  p.node('aw-tunnel-replace-dns').checked = true;
  await p.click('aw-tunnel-create-btn');
  assert.deepEqual(p.sent.find((c) => c[1] === 'tunnel_create'), ['anywhere', 'tunnel_create', 'api_token:' + API_TOKEN, 'hostname:music.example.com', 'replace_dns:1', 'csrf:' + p.TOKEN]);
  assert.equal(p.node('aw-tunnel-api-token').value, '', 'API token emptied, even after a refusal');
  assert.match(p.node('aw-message').textContent, /no zone on this account/);
  p.node('aw-tunnel-api-token').value = 'too short';
  const n = p.sent.length;
  await p.click('aw-tunnel-create-btn');
  assert.equal(p.sent.length, n, 'a bad API token is not sent');
});

test('one tunnel command at a time: the buttons are off while one is in flight', async () => {
  const p = fakePage({ ...GATEWAY, tunnel_status: { ok: 1, data: S({ state: 'not_installed', installed_version: null }) }, tunnel_install: { ok: 1, data: {} } });
  await p.ready();
  assert.ok(p.isShown('aw-tunnel-install'));
  const release = p.holdCommands();
  const first = p.click('aw-tunnel-install');
  await p.settle();
  assert.equal(p.node('aw-tunnel-install').disabled, true, 'disabled while in flight');
  await p.click('aw-tunnel-install');
  assert.equal(p.sent.filter((c) => c[1] === 'tunnel_install').length, 1, 'a second click sends nothing');
  release();
  await first;
  await p.settle();
  assert.equal(p.node('aw-tunnel-install').disabled, false, 'enabled again');
  assert.deepEqual(p.sent.find((c) => c[1] === 'tunnel_install'), ['anywhere', 'tunnel_install', 'csrf:' + p.TOKEN]);
});

test('Remove asks first, and says the tunnel token is deleted', async () => {
  const p = fakePage({ ...GATEWAY, tunnel_status: { ok: 1, data: S({ state: 'stopped', mode: 'registered', hostname: 'music.example.com' }) }, tunnel_remove: { ok: 1, data: {} } });
  await p.ready();
  const remove = p.created.find((e) => e.tagName === 'BUTTON' && e.attrs['data-aw-key'] === 'tunnel:remove');
  assert.ok(remove && !remove.hasAttribute('hidden'), 'Remove shown');
  assert.ok(p.isShown('aw-tunnel-remove-note'), 'with its note');
  await p.click(remove);
  assert.ok(!p.sent.some((c) => c[1] === 'tunnel_remove'), 'the first click only asks');
  assert.match(remove.textContent, /delete the tunnel token/);
  await p.click(remove);
  assert.deepEqual(p.sent.find((c) => c[1] === 'tunnel_remove'), ['anywhere', 'tunnel_remove', 'csrf:' + p.TOKEN]);
  assert.match(p.node('aw-message').textContent, /tunnel token are deleted/);
});

test('the temporary tunnel: link, QR code, countdown, Stop; pairing codes follow it', async () => {
  const quick = S({ state: 'connected', mode: 'quick', public_url: 'https://a-b-c.trycloudflare.com', connections: 2, quick_ends_in: 3540, uptime: 60 });
  const p = fakePage({
    ...GATEWAY,
    users: { data: { users: [{ username: 'alice', role: 'listener', totp_enabled: 0, disabled: 0, downloads: false }] } },
    grants: { data: { players: [] } },
    tunnel_status: { ok: 1, data: quick },
    tunnel_stop: { ok: 1, data: {} },
    pairing: { ok: 1, data: { code: 'ABCDEFGHJK', expires_in: 600 } },
  });
  await p.ready();
  assert.ok(p.isShown('aw-tunnel-quick-box') && p.isShown('aw-tunnel-quick-run') && p.isShown('aw-tunnel-quick-stop'));
  assert.ok(!p.isShown('aw-tunnel-quick-btn'), 'no second start while it runs');
  assert.equal(p.node('aw-tunnel-quick-link').attrs.href, 'https://a-b-c.trycloudflare.com');
  assert.equal(p.node('aw-tunnel-quick-link').textContent, 'https://a-b-c.trycloudflare.com');
  const svg = p.node('aw-tunnel-quick-qr').children[0];
  assert.equal(svg && svg.tagName, 'SVG', 'a QR code');
  assert.equal(svg.attrs['aria-label'], 'QR code for https://a-b-c.trycloudflare.com');
  assert.match(p.node('aw-tunnel-quick-countdown').textContent, /^Stops in 5[89]:\d\d\.$/);
  assert.ok(p.intervals.some(([, ms]) => ms === 1000), 'the countdown ticks');
  // A pairing code made now points at the temporary address.
  const add = p.created.find((e) => e.tagName === 'BUTTON' && e.attrs['aria-label'] === 'Add a device for alice');
  await p.click(add);
  const pairQr = p.created.filter((e) => e.tagName === 'PATH').length;
  assert.ok(pairQr >= 2, 'a pairing QR code was drawn');
  assert.ok(!p.node('aw-pair').textContent.includes('No QR code'), 'with the tunnel address standing in');
  await p.click('aw-tunnel-quick-stop');
  assert.deepEqual(p.sent.find((c) => c[1] === 'tunnel_stop'), ['anywhere', 'tunnel_stop', 'csrf:' + p.TOKEN]);
});

test('Copy log: fetched with the token, copied as text; selected when the clipboard is refused', async () => {
  const p = fakePage({ ...GATEWAY, tunnel_status: { ok: 1, data: S({ log: ['a'] }) }, tunnel_log: { ok: 1, data: { log: ['one', HOSTILE] } } });
  await p.ready();
  await p.click('aw-tunnel-copy-log');
  assert.deepEqual(p.sent.find((c) => c[1] === 'tunnel_log'), ['anywhere', 'tunnel_log', 'csrf:' + p.TOKEN]);
  assert.deepEqual(p.copied, ['one\n' + HOSTILE]);
  assert.equal(p.node('aw-message').textContent, 'Log copied.');
  // No clipboard (e.g. plain http): the log is shown and selected instead.
  p.navigator.clipboard.writeText = async () => { throw new Error('not allowed'); };
  const selected = [];
  p.ctx.getSelection = () => ({ selectAllChildren: (n) => selected.push(n) });
  await p.click('aw-tunnel-copy-log');
  assert.deepEqual(selected, [p.node('aw-tunnel-log')], 'the log is selected');
  assert.equal(p.node('aw-tunnel-log').textContent, 'one\n' + HOSTILE);
  assert.match(p.node('aw-message').textContent, /couldn\u2019t be copied automatically, so it is selected/);
});

test('the page has the section, ASCII only, with the scopes and warnings', () => {
  const html = readFileSync(new URL('basic.html', dir), 'utf8');
  for (const id of ['aw-h-tunnel', 'aw-tunnel', 'aw-tunnel-state', 'aw-tunnel-info', 'aw-tunnel-actions', 'aw-tunnel-install', 'aw-tunnel-update', 'aw-tunnel-start', 'aw-tunnel-stop',
    'aw-tunnel-remove-note', 'aw-tunnel-token-form', 'aw-tunnel-token', 'aw-tunnel-token-host', 'aw-tunnel-register-btn', 'aw-tunnel-create-form', 'aw-tunnel-api-token',
    'aw-tunnel-create-host', 'aw-tunnel-replace-dns', 'aw-tunnel-create-btn', 'aw-tunnel-quick-box', 'aw-tunnel-quick-run', 'aw-tunnel-quick-link', 'aw-tunnel-quick-qr',
    'aw-tunnel-quick-countdown', 'aw-tunnel-quick-btn', 'aw-tunnel-quick-stop', 'aw-tunnel-log-box', 'aw-tunnel-log', 'aw-tunnel-copy-log', 'aw-tunnel-override-url', 'aw-tunnel-override-header']) {
    assert.ok(html.includes(`id="${id}"`), id);
    assert.ok(src.includes(`'${id}'`) || ['aw-tunnel-info', 'aw-tunnel-actions'].includes(id) || src.includes(id), `${id} is used by the script`);
  }
  assert.match(html, /id="aw-tunnel-state"[^>]*aria-live="polite"/, 'the state is announced');
  assert.match(html, /<input id="aw-tunnel-api-token" type="password"/, 'the API token is a password field');
  for (const id of ['aw-tunnel-token', 'aw-tunnel-token-host', 'aw-tunnel-api-token', 'aw-tunnel-create-host', 'aw-tunnel-replace-dns']) {
    assert.ok(html.includes(`<label for="${id}">`), `${id} is labelled`);
  }
  const scopes = html.indexOf('Account &rsaquo; Cloudflare Tunnel &rsaquo; Edit');
  assert.ok(scopes > 0 && html.includes('Zone &rsaquo; DNS &rsaquo; Edit') && html.includes('Zone &rsaquo; Zone &rsaquo; Read'), 'the three scopes');
  assert.ok(scopes < html.indexOf('id="aw-tunnel-api-token"'), 'listed above the API token field');
  assert.ok(html.includes('The API token is used once, for setting up the tunnel, and is never stored.'));
  assert.ok(html.includes('Test only. The address is random, Cloudflare gives no guarantee for it, and this plugin stops it after one hour. For everyday use, register a tunnel below.'));
  assert.match(html, /Anyone who has the address can reach this server&rsquo;s sign-in page until the temporary tunnel stops\./);
  assert.ok(html.includes('href="https://www.cloudflare.com/terms/"'), 'the terms are linked');
  assert.match(html, /deletes cloudflared and the stored tunnel token/);
  assert.ok(html.indexOf('id="aw-tunnel-quick-box"') < html.indexOf('id="aw-tunnel-token-form"'), 'the temporary tunnel comes before the forms ("register a tunnel below")');
  assert.ok(!/[^\x00-\x7f]/.test(html), 'ASCII only');
});
