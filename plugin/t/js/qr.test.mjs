// The settings page's QR encoder (settings/qr.js): a generated copy of the web app's
// encoder, checked to be current, to load as a classic script, and to match the qrcode
// crate's reference symbols (web/app/tests/fixtures/qr-vectors.json).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { pluginCopy, SOURCE, TARGET } from '../../../tools/sync-qr.mjs';
import { decodeQr } from '../../../web/app/tests/helpers/qr-decode.mjs';

const copy = readFileSync(TARGET, 'utf8');
const { vectors } = JSON.parse(readFileSync(new URL('../../../web/app/tests/fixtures/qr-vectors.json', import.meta.url), 'utf8'));

function load() {
  const ctx = { URL };
  ctx.globalThis = ctx;
  vm.runInNewContext(copy, ctx);
  return ctx.AnywhereQR;
}

test('the plugin copy is current (run node tools/sync-qr.mjs after changing the encoder)', () => {
  assert.equal(copy, pluginCopy(readFileSync(SOURCE, 'utf8')));
});

test('is ASCII only, uses no HTML-parsing APIs, and only sets AnywhereQR', () => {
  assert.ok(!/[^\x00-\x7f]/.test(copy), `non-ASCII at ${copy.search(/[^\x00-\x7f]/)}`);
  assert.ok(!/innerHTML|outerHTML|insertAdjacentHTML|document\.write|\beval\s*\(|new\s+Function/.test(copy));
  assert.ok(!/\bfetch\s*\(|XMLHttpRequest|import\s*\(/.test(copy), 'no network access');
  const ctx = { URL };
  ctx.globalThis = ctx;
  vm.runInNewContext(copy, ctx);
  assert.deepEqual(Object.keys(ctx).sort(), ['AnywhereQR', 'URL', 'globalThis']);
  assert.ok(Object.isFrozen(ctx.AnywhereQR));
});

test('matches the reference symbols for every mask', () => {
  const QR = load();
  assert.ok(vectors.length >= 8);
  for (const v of vectors) {
    for (let mask = 0; mask < 8; mask++) {
      const qr = QR.encodeQr(v.text, { mask });
      assert.equal(qr.version, v.version);
      const rows = [];
      for (let y = 0; y < qr.size; y++) {
        const bits = Array.from(qr.modules.slice(y * qr.size, (y + 1) * qr.size));
        while (bits.length % 4) bits.push(0);
        let row = '';
        for (let i = 0; i < bits.length; i += 4) row += bits.slice(i, i + 4).reduce((a, b) => (a << 1) | b, 0).toString(16);
        rows.push(row);
      }
      assert.deepEqual(rows, v.masks[mask], `${v.text.slice(0, 20)} mask ${mask}`);
    }
  }
});

test('a pairing link decodes back intact', () => {
  const QR = load();
  const link = QR.pairingLink('https://music.example.com', 'abcde-fghjk');
  assert.equal(link, 'lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK');
  const qr = QR.encodeQr(link);
  assert.equal(decodeQr(qr.size, (x, y) => qr.modules[y * qr.size + x]).text, link);
  for (const bad of ['http://music.example.com', 'https://music.example.com/lms', 'javascript:alert(1)', '']) {
    assert.equal(QR.pairingLink(bad, 'ABCDEFGHJK'), null, bad);
  }
});
