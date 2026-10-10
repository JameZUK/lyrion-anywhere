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
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    append(...c) { this.children.push(...c); }
    replaceChildren(...c) { this._text = ''; this.children = c; }
    focus() {}
  }
  const byId = new Map();
  for (const id of ['aw-status', 'aw-users', 'aw-devices', 'aw-audit', 'aw-message', 'aw-create-btn', 'aw-new-user', 'aw-new-password', 'aw-new-role']) {
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
    users: { data: { users: [{ username: 'bob', role: 'listener', totp_enabled: 0, disabled: 0 }] } },
    grants: { data: { players: ['aa:bb:cc:dd:ee:01'] } },
    devices: { data: { devices: [{ id: 1, name: HOSTILE, username: 'bob', mac: '02:4c:41:00:00:09', last_seen: 1 }] } },
    audit: { data: { audit: [{ at: 1, event: 'login', username: 'bob', detail: HOSTILE }] } },
  };
  const fetch = async (_url, opts) => {
    const cmd = JSON.parse(opts.body).params[1];
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
});
