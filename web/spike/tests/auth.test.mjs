// Browser auth helper tests (node --test). fetch and storage are stubbed.
import { test } from 'node:test';
import assert from 'node:assert/strict';

globalThis.navigator ??= { userAgent: 'test' };
const { AuthError, defaultDeviceName, login, refresh, rememberedDevice } = await import('../auth.js');

function stubFetch(handler) {
  const calls = [];
  globalThis.fetch = async (path, init) => {
    calls.push({ path, init });
    const { status, body } = handler(path, init);
    return {
      status,
      ok: status >= 200 && status < 300,
      json: async () => body,
    };
  };
  return calls;
}

function memoryStorage() {
  const m = new Map();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, String(v)) };
}

const SESSION = { access_token: 'a', user: { username: 'u' }, device: { id: 7, name: 'n', mac: '02:4c:41:00:00:01' } };

test('every auth request carries the CSRF header and same-origin credentials', async () => {
  globalThis.localStorage = memoryStorage();
  const calls = stubFetch(() => ({ status: 200, body: SESSION }));
  await login('u', 'p');
  await refresh();
  for (const c of calls) {
    assert.equal(c.init.headers['X-Anywhere-Client'], '1', c.path);
    assert.equal(c.init.credentials, 'same-origin', c.path);
    assert.equal(c.init.method, 'POST');
  }
});

test('login reuses the remembered device and never sends a refresh token', async () => {
  globalThis.localStorage = memoryStorage();
  const calls = stubFetch(() => ({ status: 200, body: SESSION }));
  await login('u', 'p');
  assert.equal(rememberedDevice(globalThis.localStorage), 7);
  await login('u', 'p');
  const second = JSON.parse(calls[1].init.body);
  assert.equal(second.device_id, 7);
  assert.equal(second.refresh, undefined, 'browsers must keep the refresh token in the cookie');
});

test('failures surface the status without leaking detail', async () => {
  globalThis.localStorage = memoryStorage();
  stubFetch(() => ({ status: 429, body: { error: 'too many attempts; try again later' } }));
  await assert.rejects(login('u', 'p'), (e) => e instanceof AuthError && e.status === 429);
});

test('blocked storage does not break sign-in', async () => {
  globalThis.localStorage = {
    getItem() {
      throw new Error('blocked');
    },
    setItem() {
      throw new Error('blocked');
    },
  };
  stubFetch(() => ({ status: 200, body: SESSION }));
  const s = await login('u', 'p');
  assert.equal(s.device.id, 7);
  assert.equal(rememberedDevice(globalThis.localStorage), undefined);
});

test('remembered device ids are validated', () => {
  for (const bad of ['', 'abc', '-1', '0', '1.5', '1e400']) {
    const s = memoryStorage();
    s.setItem('anywhere.device', bad);
    assert.equal(rememberedDevice(s), undefined, bad);
  }
});

test('default device names are readable', () => {
  assert.equal(defaultDeviceName('Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0'), 'Firefox on Linux');
  assert.equal(defaultDeviceName('Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 Chrome/140.0 Mobile Safari/537.36'), 'Chrome on Android');
  assert.equal(defaultDeviceName('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1'), 'Safari on iOS');
});
