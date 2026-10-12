// "Add a device" on the settings page: a one-time code from the gateway, shown large and
// as a QR code holding the pairing link, with its time left. Admin only (the CSRF token),
// and nothing from the gateway's reply is ever treated as markup.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { decodeQr } from '../../../web/app/tests/helpers/qr-decode.mjs';

const dir = new URL('../../Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/', import.meta.url);
const src = readFileSync(new URL('anywhere.js', dir), 'utf8');
const qrSrc = readFileSync(new URL('qr.js', dir), 'utf8');

const helpers = (() => {
  const ctx = { __anywhereTest: {}, URL };
  ctx.globalThis = ctx;
  vm.runInNewContext(qrSrc, ctx);
  vm.runInNewContext(src, ctx);
  return { t: ctx.__anywhereTest, QR: ctx.AnywhereQR };
})();

test('countdown text', () => {
  const { t } = helpers;
  assert.equal(t.formatCountdown(600), '10:00');
  assert.equal(t.formatCountdown(65), '1:05');
  assert.equal(t.formatCountdown(0), '0:00');
  assert.equal(t.formatCountdown(-5), '0:00');
  assert.equal(t.formatCountdown('x'), '0:00');
});

test('what the panel shows for a reply', () => {
  const { t, QR } = helpers;
  const v = t.pairingView({ code: 'ABCDEFGHJK', expires_in: 600 }, 'https://music.example.com', QR);
  assert.equal(v.code, 'ABCDE-FGHJK');
  assert.equal(v.link, 'lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK');
  assert.equal(v.seconds, 600);
  // No usable public URL: the code alone, and the page says why.
  for (const url of ['', 'http://music.example.com', 'https://music.example.com/lms', 'javascript:alert(1)']) {
    const w = t.pairingView({ code: 'ABCDEFGHJK', expires_in: 600 }, url, QR);
    assert.equal(w.link, null, url);
    assert.equal(w.publicUrlSet, url !== '');
  }
  // Odd expiry values fall back to the gateway's 10 minutes.
  for (const e of [undefined, 0, -1, 'x', 1e9]) {
    assert.equal(t.pairingView({ code: 'ABCDEFGHJK', expires_in: e }, '', QR).seconds, 600, String(e));
  }
  // Anything that isn't a code is not shown at all.
  for (const code of [undefined, null, 42, '', '<img src=x onerror=alert(1)>', 'ABCD&server=https://evil.example', 'A'.repeat(40)]) {
    assert.equal(t.pairingView({ code, expires_in: 600 }, 'https://music.example.com', QR), null, String(code));
  }
  assert.equal(t.pairingView(null, '', QR), null);
  assert.equal(t.pairingView({ code: 'ABCDEFGHJK' }, '', undefined), null, 'no encoder, no view');
});

/** A tiny DOM (as in settings.test.mjs), plus SVG elements and timers under test control. */
function fakePage(publicUrl, reply) {
  const created = [];
  class Node {
    constructor(tag, ns) {
      this.tagName = tag.toUpperCase();
      this.ns = ns || null;
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
    insertAdjacentHTML() { throw new Error('insertAdjacentHTML used'); }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
    removeAttribute(k) { delete this.attrs[k]; }
    hasAttribute(k) { return k in this.attrs; }
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    append(...c) { this.children.push(...c); }
    replaceChildren(...c) { this._text = ''; this.children = c; }
    focus() {}
  }
  const byId = new Map();
  for (const id of ['anywhere-app', 'aw-status', 'aw-users', 'aw-pair', 'aw-devices', 'aw-audit', 'aw-message', 'aw-create-btn', 'aw-new-user', 'aw-new-password', 'aw-new-role', 'aw-session-days', 'aw-session-btn', 'aw-plugins', 'aw-plugins-btn']) {
    byId.set(id, new Node('div'));
  }
  const TOKEN = 'ef'.repeat(16);
  byId.get('anywhere-app').setAttribute('data-csrf', TOKEN);
  byId.get('anywhere-app').setAttribute('data-public-url', publicUrl);
  byId.get('aw-pair').setAttribute('hidden', '');
  let listener;
  const document = {
    createElement: (tag) => new Node(tag),
    createElementNS: (ns, tag) => new Node(tag, ns),
    getElementById: (id) => byId.get(id),
    addEventListener: (_type, fn) => { listener = fn; },
  };
  const replies = {
    status: { running: 1, enabled: 1, pid: 1, uptime: 1, listen: '127.0.0.1:9443', binary_ok: 1 },
    users: { data: { users: [{ username: 'alice', role: 'listener', totp_enabled: 0, disabled: 0, downloads: false }] } },
    grants: { data: { players: [] } },
    devices: { data: { devices: [] } },
    audit: { data: { audit: [] } },
    settings: { data: {} },
    plugins: { data: { plugins: [] } },
    pairing: reply,
  };
  const sent = [];
  const fetch = async (_url, opts) => {
    const cmd = JSON.parse(opts.body).params[1];
    sent.push(cmd);
    return { json: async () => ({ result: cmd[0] === 'players' ? {} : replies[cmd[1]] || {} }) };
  };
  const timers = [];
  let now = 1_000_000;
  const FakeDate = class extends Date {
    static now() { return now; }
  };
  const ctx = {
    document, fetch, URL, Date: FakeDate,
    setInterval: (fn) => { timers.push(fn); return timers.length; },
    clearInterval: (id) => { timers[id - 1] = null; },
    setTimeout: () => 0,
  };
  ctx.globalThis = ctx;
  vm.runInNewContext(qrSrc, ctx);
  vm.runInNewContext(src, ctx);
  return {
    byId, created, sent, TOKEN,
    ready: () => listener(),
    advance: (ms) => { now += ms; for (const f of timers) if (f) f(); },
  };
}

const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); };

async function openPanel(page) {
  page.ready();
  await settle();
  const button = page.created.find((e) => e.tagName === 'BUTTON' && e.textContent === 'Add a device');
  assert.ok(button, 'Add a device button shown');
  for (const fn of button.listeners.click) fn();
  await settle();
  return page.byId.get('aw-pair');
}

test('shows the code, a QR code of the pairing link and the time left', async () => {
  const page = fakePage('https://music.example.com', { ok: 1, data: { code: 'ABCDEFGHJK', expires_in: 600 } });
  const panel = await openPanel(page);
  assert.deepEqual(page.sent.find((c) => c[1] === 'pairing'), ['anywhere', 'pairing', 'username:alice', 'csrf:' + page.TOKEN], 'sent with the page token');
  assert.equal(panel.getAttribute('hidden'), null, 'panel shown');
  const text = panel.textContent;
  assert.ok(text.includes('Add a device for alice'));
  assert.ok(text.includes('ABCDE-FGHJK'), 'code shown, grouped');
  assert.ok(text.includes('Expires in 10:00'));
  assert.equal(page.byId.get('aw-message').textContent, '', 'the code is not put in the status line');

  const svg = page.created.find((e) => e.tagName === 'SVG');
  assert.equal(svg.ns, 'http://www.w3.org/2000/svg');
  assert.equal(svg.attrs.role, 'img');
  const path = page.created.find((e) => e.tagName === 'PATH');
  // Rebuild the modules from the path and decode them: the QR holds the pairing link.
  const n = Number(svg.attrs.viewBox.split(' ')[2]);
  const size = n - 8;
  const dark = new Set();
  for (const m of path.attrs.d.matchAll(/M(\d+) (\d+)h(\d+)/g)) {
    for (let k = 0; k < Number(m[3]); k++) dark.add(`${Number(m[1]) - 4 + k},${Number(m[2]) - 4}`);
  }
  const decoded = decodeQr(size, (x, y) => (dark.has(`${x},${y}`) ? 1 : 0));
  assert.equal(decoded.text, 'lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK');

  page.advance(65_000);
  assert.ok(panel.textContent.includes('Expires in 8:55'));
  page.advance(600_000);
  assert.ok(panel.textContent.includes('has expired'));
  const qrBox = page.created.find((e) => e.className === 'aw-qr');
  assert.equal(qrBox.getAttribute('hidden'), '', 'the QR code goes when it expires');

  const done = page.created.find((e) => e.tagName === 'BUTTON' && e.textContent === 'Done');
  for (const fn of done.listeners.click) fn();
  assert.equal(panel.getAttribute('hidden'), '');
  assert.equal(panel.children.length, 0);
});

test('without a usable public URL: the code and a clear message, no QR code', async () => {
  for (const url of ['', 'http://music.example.com']) {
    const page = fakePage(url, { ok: 1, data: { code: 'ABCDEFGHJK', expires_in: 600 } });
    const panel = await openPanel(page);
    assert.ok(panel.textContent.includes('ABCDE-FGHJK'));
    assert.ok(/No QR code: (set the Public URL|the Public URL above must be an https)/.test(panel.textContent), url);
    assert.equal(page.created.filter((e) => e.tagName === 'SVG').length, 0);
  }
});

test('a hostile or failed reply shows no code and no markup', async () => {
  for (const reply of [
    { ok: 1, data: { code: '<img src=x onerror=alert(1)>', expires_in: 600 } },
    { ok: 1, data: {} },
    { error: 'refused: use the Lyrion Anywhere settings page' },
  ]) {
    const page = fakePage('https://music.example.com', reply);
    const panel = await openPanel(page);
    assert.equal(panel.getAttribute('hidden'), '', 'panel stays hidden');
    assert.ok(page.byId.get('aw-message').textContent.length > 0, 'the problem is reported');
    assert.equal(page.created.filter((e) => e.tagName === 'SVG' || e.tagName === 'IMG' || e.tagName === 'SCRIPT').length, 0);
  }
});
