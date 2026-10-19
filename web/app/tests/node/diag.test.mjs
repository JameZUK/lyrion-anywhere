// Connection-log lines for the protocol and the output's back-pressure (node --test).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  Persisting,
  STAT_GLOSS,
  describeDiag,
  describeDrained,
  describeNotConsuming,
  describeOutputChange,
  describeStat,
  describeStrm,
  outputCause,
  outputName,
  secs,
} from '../../static/player/diag.js';

test('strm lines: the command letter, and for s its autostart, format and threshold only', () => {
  assert.equal(describeStrm({ type: 'strm', command: 's', autostart: '1', format: 'f', threshold: 255 }), 'lms: strm s autostart=1 format=f threshold=255');
  for (const c of ['p', 'u', 'q', 'f', 'a']) assert.equal(describeStrm({ type: 'strm', command: c, autostart: '0', format: 'm', threshold: 1 }), `lms: strm ${c}`);
});

test('strm lines never carry anything but single printable characters and a small number', () => {
  // Whatever the fields hold (a confused core, a hostile server), the line stays short and
  // plain: no URLs, no control characters, no injected text.
  const junk = ['', 'ss', '\n', ' ', 'http://10.0.0.10/', '\u0000', 'é', 12, null, undefined, {}, []];
  for (const j of junk) {
    const line = describeStrm({ command: 's', autostart: j, format: j, threshold: j });
    assert.match(line, /^lms: strm s autostart=[!-~] format=[!-~] threshold=(\d{1,3}|\?)$/, String(j));
    assert.match(describeStrm({ command: j }), /^lms: strm [!-~]$/, String(j));
  }
  for (const t of [-1, 256, 1.5, NaN, '255']) assert.match(describeStrm({ command: 's', autostart: '1', format: 'f', threshold: t }), /threshold=\?$/);
  assert.equal(describeStrm(null), 'lms: strm ?');
});

test('STAT lines carry a short gloss for each kind, and nothing unknown', () => {
  assert.equal(describeStat('STMl'), 'player: STMl (loaded, waiting for LMS to start it)');
  for (const e of ['STMl', 'STMs', 'STMp', 'STMr', 'STMo', 'STMu', 'STMd', 'STMf', 'STMc', 'STMn']) {
    assert.ok(STAT_GLOSS[e], e);
    assert.match(describeStat(e), new RegExp(`^player: ${e} \\([a-zA-Z ,]+\\)$`));
  }
  assert.equal(describeStat('STMz'), 'player: STMz');
  for (const e of ['STAT', 'STM', 'STMlx', 'stml', 'STM\n', '', null, 5, { toString: () => 'STMl' }]) assert.equal(describeStat(e), null, String(e));
});

test('diagnostics events become lines with a kind each, for rate limiting', () => {
  assert.deepEqual(describeDiag({ type: 'strm', command: 'q' }), { line: 'lms: strm q', kind: 'strm:q' });
  assert.deepEqual(describeDiag({ type: 'stat', event: 'STMp' }), { line: 'player: STMp (paused, as LMS asked)', kind: 'stat:STMp' });
  assert.equal(describeDiag({ type: 'stat', event: 'bogus' }), null);
  assert.deepEqual(describeDiag({ type: 'flac_header', rate: 44100, channels: 2, bits: 16 }), {
    line: 'player: the stream had no FLAC header (a seek is sent so); made one from its first frame: 44100 Hz, 16-bit, 2 ch',
    kind: undefined,
  });
  assert.equal(describeDiag({ type: 'flac_header', rate: NaN, channels: -1, bits: 'x' }).line, 'player: the stream had no FLAC header (a seek is sent so); made one from its first frame: ? Hz, ?-bit, ? ch');
  assert.equal(describeDiag({ type: 'other' }), null);
  assert.equal(describeDiag(undefined), null);
});

test('output states by the names the core uses', () => {
  assert.equal(outputName('Playing'), 'Playing');
  assert.equal(outputName('Paused { until: None }'), 'Paused');
  assert.equal(outputName('Paused { until: Some(1234) }'), 'Paused');
  assert.equal(outputName('Held'), 'Held');
  assert.equal(outputName(''), '?');
  assert.equal(outputName(undefined), '?');
  assert.equal(describeOutputChange('Playing', 'Paused', 'after strm p'), 'player: Playing → Paused (after strm p)');
  assert.equal(describeOutputChange('Stopped', 'Waiting', null), 'player: Stopped → Waiting');
});

test('the cause of an output change, where it is known', () => {
  const strm = (command, at) => ({ command, at });
  assert.equal(outputCause('Playing', 'Paused', 'Paused { until: None }', strm('p', 1000), 1100), 'after strm p');
  assert.equal(outputCause('Playing', 'Held', 'Playing', strm('p', 1000), 1100), 'local pause');
  assert.equal(outputCause('Held', 'Playing', 'Playing', null, 1100), 'local resume');
  // A pause that ends by itself: a sync pause, or a timed start.
  assert.equal(outputCause('Playing', 'Paused', 'Paused { until: Some(5) }', strm('p', 1000), 1100), 'sync');
  assert.equal(outputCause('Loaded', 'Paused', 'Paused { until: Some(5) }', strm('u', 1000), 1100), 'timed start, after strm u');
  assert.equal(outputCause('Paused', 'Playing', 'Playing', strm('u', 0), 60_000), 'timed pause ended');
  assert.equal(outputCause('Waiting', 'Playing', 'Playing', strm('s', 0), 60_000), 'buffer ready');
  // A strm long ago is not the cause.
  assert.equal(outputCause('Playing', 'Stopped', 'Stopped', strm('q', 0), 60_000), null);
  assert.equal(outputCause('Playing', 'Stopped', 'Stopped', strm('q', 59_000), 60_000), 'after strm q');
});

test('a condition is reported once when it has lasted long enough, and once when it ends', () => {
  const p = new Persisting(10_000);
  assert.equal(p.update(true, 0), null);
  assert.equal(p.update(true, 9_999), null);
  assert.equal(p.update(true, 10_000), 'began');
  assert.equal(p.update(true, 20_000), null, 'once');
  assert.equal(p.update(false, 25_000), 'ended');
  assert.equal(p.lasted, 25_000);
  assert.equal(p.update(false, 26_000), null);
  // A short one is never reported, at either end.
  assert.equal(p.update(true, 30_000), null);
  assert.equal(p.update(false, 31_000), null);
  // Reset forgets it without an end.
  p.update(true, 40_000);
  assert.equal(p.update(true, 50_000), 'began');
  p.reset();
  assert.equal(p.update(false, 51_000), null);
  // A zero limit reports at once.
  const z = new Persisting(0);
  assert.equal(z.update(true, 5), 'began');
});

test('back-pressure and drain lines', () => {
  assert.equal(describeNotConsuming(8, 14.32), 'media: element not consuming (lead 8.0 s, buffered 14.3 s)');
  assert.equal(describeNotConsuming(NaN, -3), 'media: element not consuming (lead 0.0 s, buffered 0.0 s)');
  assert.equal(describeDrained('played-out'), 'media: drained, counted as played out');
  assert.equal(describeDrained('flushed', 1152), 'media: drained, flushed 1152 pending frames');
  assert.equal(describeDrained('flushed', 'x'), 'media: drained, flushed 0 pending frames');
  assert.equal(describeDrained('ignored', 'output Paused'), 'media: drained, ignored (output Paused)');
  assert.equal(secs(61.25), '61.3');
  assert.equal(secs(Infinity), '0.0');
});

test('the worker logs the core diagnostics, output changes and back-pressure', () => {
  const src = readFileSync(new URL('../../static/player/player-worker.js', import.meta.url), 'utf8');
  assert.match(src, /player\.set_diagnostics\(true\)/);
  assert.match(src, /player\.take_diagnostics\(\)/);
  assert.match(src, /noteOutput\(output, status\.output\)/);
  assert.match(src, /describeNotConsuming\(/);
  assert.match(src, /describeDrained\('played-out'\)/);
  assert.match(src, /undecoded: \(\) =>/);
  // Loaded with the build version, like every other helper.
  assert.match(src, /import\(`\.\/diag\.js\$\{VQ\}`\)/);
});
