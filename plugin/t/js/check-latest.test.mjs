// M17: LMS installs find plugin updates at releases/latest/download/repo.xml. GitHub's
// "latest" is the newest full release, so promoting an Android pre-release would make that
// URL a 404 and every install would silently stop seeing updates. tools/check-latest.sh
// checks the URL serves the plugin's repo.xml; CI runs it after every release and weekly.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const SCRIPT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../tools/check-latest.sh');

const repoXml = (version, name = 'LyrionAnywhere') => `<?xml version="1.0" encoding="UTF-8"?>
<extensions>
	<details><title lang="EN">Lyrion Anywhere</title></details>
	<plugins>
		<plugin name="${name}" version="${version}" minTarget="9.0" maxTarget="*">
			<title lang="EN">Lyrion Anywhere</title>
			<url>https://example.com/releases/download/v${version}/${name}-${version}.zip</url>
			<sha>${'a'.repeat(40)}</sha>
		</plugin>
	</plugins>
</extensions>
`;

async function serve(routes) {
  const server = createServer((req, res) => {
    const r = routes[req.url];
    if (!r) { res.writeHead(404, { 'content-type': 'text/plain' }); res.end('Not Found'); return; }
    res.writeHead(r.status ?? 200, { 'content-type': r.type ?? 'application/xml' });
    res.end(r.body);
  });
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  const base = `http://127.0.0.1:${server.address().port}`;
  return { base, close: () => new Promise((ok) => server.close(ok)) };
}

function run(args) {
  return new Promise((resolve) => {
    execFile('bash', [SCRIPT, ...args], { env: { ...process.env, CHECK_LATEST_DELAY: '0' } }, (err, stdout, stderr) => {
      resolve({ code: err ? err.code : 0, stdout, stderr });
    });
  });
}

test('the plugin\'s repo.xml passes, and its version is reported', async () => {
  const s = await serve({ '/repo.xml': { body: repoXml('0.4.0') } });
  try {
    const r = await run([`${s.base}/repo.xml`]);
    assert.equal(r.code, 0, r.stderr);
    assert.match(r.stdout, /0\.4\.0/);
    const ok = await run(['--expect-version', '0.4.0', `${s.base}/repo.xml`]);
    assert.equal(ok.code, 0, ok.stderr);
  } finally { await s.close(); }
});

test('a 404 (an Android release marked Latest) fails loudly, saying why', async () => {
  const s = await serve({});
  try {
    const r = await run([`${s.base}/repo.xml`]);
    assert.notEqual(r.code, 0);
    assert.match(r.stderr, /android-v\* release was marked Latest/, 'names the likely cause');
    assert.match(r.stderr, /gh release edit v<version> --latest/, 'and the remedy');
  } finally { await s.close(); }
});

test('anything that is not the plugin\'s repo.xml fails', async () => {
  const s = await serve({
    '/html': { type: 'text/html', body: '<!DOCTYPE html><html><body>Not here</body></html>' },
    '/other': { body: repoXml('1.0.0', 'SomethingElse') },
    '/broken': { body: '<extensions><plugins><plugin name="LyrionAnywhere"' },
    '/empty': { body: '' },
    '/badver': { body: repoXml('0.4.0;rm') },
    '/error': { status: 500, body: repoXml('0.4.0') },
  });
  try {
    for (const p of ['/html', '/other', '/broken', '/empty', '/badver', '/error']) {
      const r = await run([`${s.base}${p}`]);
      assert.notEqual(r.code, 0, `${p} refused`);
    }
  } finally { await s.close(); }
});

test('a different version from the one just released fails', async () => {
  const s = await serve({ '/repo.xml': { body: repoXml('0.3.0') } });
  try {
    const r = await run(['--expect-version', '0.4.0', `${s.base}/repo.xml`]);
    assert.notEqual(r.code, 0);
    assert.match(r.stderr, /0\.3\.0/);
  } finally { await s.close(); }
});

test('the default URL is the documented repository URL', async () => {
  const { readFile } = await import('node:fs/promises');
  const text = await readFile(SCRIPT, 'utf8');
  assert.ok(text.includes('https://github.com/JameZUK/lyrion-anywhere/releases/latest/download/repo.xml'));
});
