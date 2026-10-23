// The build step that keeps the CSP strict: inline scripts are moved to files, and any
// inline script, event-handler attribute or javascript: URL left behind fails the build.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync, readdirSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const script = new URL('../../scripts/externalise-boot.mjs', import.meta.url).pathname;
function run(html, name = 'index.html') {
	const dir = mkdtempSync(join(tmpdir(), 'boot-'));
	mkdirSync(join(dir, 'sub'), { recursive: true });
	writeFileSync(join(dir, name), html);
	try {
		const out = execFileSync('node', [script, dir], { encoding: 'utf8', stdio: 'pipe' });
		return { ok: true, out, dir, html: readFileSync(join(dir, name), 'utf8') };
	} catch (e) {
		return { ok: false, out: String(e.stderr), dir };
	}
}

test('moves an inline start-up script to a file and keeps it in place', () => {
	const r = run('<html><body><div id="a"><script>{ start(document.currentScript.parentElement) }</script></div></body></html>');
	assert.ok(r.ok, r.out);
	assert.match(r.html, /<div id="a"><script src="\.\/_app\/boot\.[0-9a-f]{16}\.js"><\/script><\/div>/);
	const [file] = readdirSync(join(r.dir, '_app'));
	assert.equal(readFileSync(join(r.dir, '_app', file), 'utf8'), '{ start(document.currentScript.parentElement) }');
});

test("SvelteKit 3's start-up script (init, then start; a module service worker) moves out whole", () => {
	const body = `
		{
			__sveltekit_x = { base: new URL('.', location).pathname.slice(0, -1), version: "1" };
			const element = document.currentScript.parentElement;
			import("/_app/immutable/entry/start.A.js").then(async (kit) => {
				kit.init(__sveltekit_x);
				const app = await import("/_app/immutable/entry/app.B.js");
				kit.start(app, element);
			});
			if ('serviceWorker' in navigator) {
				addEventListener('load', function () { navigator.serviceWorker.register('/service-worker.js', { type: 'module' }); });
			}
		}`;
	const r = run(`<html><head><link href="/_app/immutable/entry/payload.C.js" rel="modulepreload"></head><body><div style="display: contents"><script>${body}</script></div></body></html>`);
	assert.ok(r.ok, r.out);
	assert.match(r.out, /1 inline script\(s\) moved; 1 page\(s\) clean/);
	assert.doesNotMatch(r.html, /<script(?![^>]*\bsrc=)[^>]*>/);
	assert.match(r.html, /<div style="display: contents"><script src="\.\/_app\/boot\.[0-9a-f]{16}\.js"><\/script><\/div>/);
	const [file] = readdirSync(join(r.dir, '_app'));
	assert.equal(readFileSync(join(r.dir, '_app', file), 'utf8'), body);
});

test('pages in subdirectories get a correct relative path', () => {
	const r = run('<script>x()</script>', 'sub/page.html');
	assert.ok(r.ok, r.out);
	assert.match(r.html, /src="\.\/\.\.\/_app\/boot\.[0-9a-f]{16}\.js"/);
});

test('external scripts are left alone', () => {
	const r = run('<script type="module" src="/_app/x.js"></script>');
	assert.ok(r.ok, r.out);
	assert.equal(r.html, '<script type="module" src="/_app/x.js"></script>');
});

test('event-handler attributes fail the build', () => {
	for (const html of ['<img src=x onerror="alert(1)">', '<body onload=go()>', '<a href=# ONCLICK = "x">']) {
		const r = run(html);
		assert.equal(r.ok, false, html);
		assert.match(r.out, /event-handler attribute/);
	}
});

test('javascript: URLs fail the build', () => {
	const r = run('<a href="javascript:alert(1)">x</a>');
	assert.equal(r.ok, false);
	assert.match(r.out, /javascript: URL/);
});

test('non-JavaScript inline script types are refused, not silently moved', () => {
	const r = run('<script type="text/x-template">{{x}}</script>');
	assert.equal(r.ok, false);
});
