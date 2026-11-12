// Review M19: a failure while the player worker starts left this device in "starting". The
// worker's message handler is async: a rejection in it (the WebAssembly core failing to load,
// say) never reaches Worker.onerror, so the page heard nothing and "Play on this device" did
// nothing until a reload. These run the real worker script in Node, against a fake core.
//
// The player files are copied into a throwaway directory beside a fake `pkg/`, so the core
// the worker loads is the test's, whatever is built (each test file runs in its own process).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const src = new URL('../../static/player/', import.meta.url);
const dir = mkdtempSync(join(tmpdir(), 'worker-start-'));
after(() => rmSync(dir, { recursive: true, force: true }));
/** The worker and the helpers it loads. */
const FILES = ['player-worker.js', 'reconnect.js', 'presentation.js', 'visual.js', 'trim.js', 'local.js', 'media-sync.js', 'diag.js', 'lead.js'];

/** The worker beside a fake core: `init` is the body of its default export (the WebAssembly start-up). */
function fakeCore(name, init) {
  const d = join(dir, name);
  mkdirSync(join(d, 'pkg'), { recursive: true });
  for (const f of FILES) cpSync(new URL(f, src), join(d, f));
  writeFileSync(join(d, 'pkg', 'anywhere_wasm.js'), `export default async () => { ${init} }\nexport class WasmPlayer {}\n`);
  return pathToFileURL(join(d, 'player-worker.js'));
}

/**
 * Load the worker as a browser would, with `self`, `onmessage` and `postMessage` as globals.
 * Returns what it posted and a way to send it a message. `extra` adds globals (WebSocket…).
 */
async function loadWorker(url, extra = {}) {
  const posted = [];
  globalThis.self = globalThis;
  globalThis.onmessage = null;
  globalThis.onunhandledrejection = null;
  globalThis.postMessage = (m) => posted.push(m);
  Object.defineProperty(globalThis, 'location', { value: { href: `${url.href}?v=test`, origin: 'https://music.example.com' }, configurable: true, writable: true });
  Object.assign(globalThis, extra);
  await import(`${url.href}?v=test`);
  return { posted, send: (data) => globalThis.onmessage({ data }) };
}

const until = async (cond, ms = 2000) => {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error('timed out');
    await new Promise((r) => setTimeout(r, 5));
  }
};

test('the core failing to load is reported to the page as fatal', async () => {
  const w = await loadWorker(fakeCore('no-core', "throw new Error('the core would not load');"));
  await w.send({ cmd: 'start', token: 't', mode: 'media' });
  await until(() => w.posted.some((m) => m.fatal));
  const f = w.posted.find((m) => m.fatal);
  assert.match(f.fatal, /the core would not load/);
  assert.ok(f.fatal.length <= 300);
  assert.ok(!w.posted.some((m) => m.started), 'never says it started');
});

test('anything else in the start branch is fatal too (the socket refused at once)', async () => {
  class RefusingSocket {
    constructor() {
      throw new DOMException('blocked', 'SecurityError');
    }
  }
  const fetch = async () => new Response('{}', { status: 200 });
  const w = await loadWorker(fakeCore('no-socket', 'return { memory: null };'), { WebSocket: RefusingSocket, fetch });
  await w.send({ cmd: 'start', token: 't', mode: 'media' });
  await until(() => w.posted.some((m) => m.fatal));
  assert.match(w.posted.find((m) => m.fatal).fatal, /blocked/);
});

test('a start that gets as far as connecting says so (the page stops its watchdog)', async () => {
  const sockets = [];
  class QuietSocket {
    constructor(url) {
      this.url = String(url);
      sockets.push(this);
    }
    send() {}
    close() {}
  }
  const fetch = async () => new Response('{}', { status: 200 });
  const w = await loadWorker(fakeCore('starts', 'return { memory: null };'), { WebSocket: QuietSocket, fetch });
  await w.send({ cmd: 'start', token: 't', mode: 'media' });
  await until(() => w.posted.some((m) => m.started));
  assert.equal(sockets.length, 1);
  assert.equal(sockets[0].url, 'wss://music.example.com/v1/player');
  assert.ok(!w.posted.some((m) => m.fatal));
});

test('an unhandled rejection is fatal while starting, and only noted once started', async () => {
  const w = await loadWorker(fakeCore('rejections', "throw new Error('x');"));
  assert.equal(typeof globalThis.onunhandledrejection, 'function', 'the worker listens for them');
  let prevented = 0;
  globalThis.onunhandledrejection({ reason: new Error('lost in a handler'), preventDefault: () => prevented++ });
  assert.match(w.posted.find((m) => m.fatal).fatal, /lost in a handler/);
  assert.equal(prevented, 1);
  // A reason that is no Error at all still reads as something.
  const n = w.posted.length;
  globalThis.onunhandledrejection({ reason: undefined });
  assert.equal(w.posted.length, n, 'fatal is said once');
});

test('the worker wires it up before anything can fail', () => {
  const worker = readFileSync(new URL('player-worker.js', src), 'utf8');
  const handler = worker.indexOf('self.onunhandledrejection');
  assert.ok(handler > 0);
  assert.ok(handler < worker.indexOf('await import('), 'set before the helpers load');
  // The start branch is guarded.
  assert.match(worker, /if \(msg\.cmd === 'start'\) \{\s*try \{/);
});
