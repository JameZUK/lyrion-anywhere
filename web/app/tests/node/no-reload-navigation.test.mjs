// In-app navigation must go through SvelteKit's router. With the hash router, assigning
// `location.hash` (or `location.href`) makes SvelteKit reload the whole page, which stops this
// device's player mid-song. Use `go()` from `#lib/nav`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = new URL('../../src/', import.meta.url).pathname;
const BANNED = [
	[/\blocation\.(?:hash|href)\s*=(?!=)/, 'location.hash/href assignment'],
	[/\blocation\.assign\s*\(/, 'location.assign'],
];
// The service worker (src/service-worker/, SvelteKit 3's layout) has no router.
const SKIP_DIR = join(ROOT, 'service-worker');

function files(dir) {
	return readdirSync(dir, { recursive: true, withFileTypes: true })
		.filter((d) => d.isFile() && /\.(svelte|ts|js)$/.test(d.name) && d.parentPath !== SKIP_DIR)
		.map((d) => join(d.parentPath, d.name));
}

test('no page-reloading navigation in the app', () => {
	const found = [];
	for (const f of files(ROOT)) {
		const code = readFileSync(f, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
		for (const [re, what] of BANNED) if (re.test(code)) found.push(`${f}: ${what}`);
	}
	assert.deepEqual(found, []);
});

// Native confirm()/alert()/prompt() show badly (or not at all) in installed web apps and
// can't be styled or made part of the page: use ConfirmDialog.
const NATIVE_DIALOG = /(?<![\w.$])(?:window\.)?(?:confirm|alert)\s*\(|\bwindow\.prompt\s*\(/;

test('no native browser dialogs in the app', () => {
	const found = [];
	for (const f of files(ROOT)) {
		const code = readFileSync(f, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
		if (NATIVE_DIALOG.test(code)) found.push(f);
	}
	assert.deepEqual(found, []);
	for (const s of ['if (!confirm("x")) return', 'window.alert(1)', 'window.prompt("name")']) assert.ok(NATIVE_DIALOG.test(s), s);
	for (const s of ['confirmTotp(e)', 'x.confirm(1)', 'async function confirmTotp(', 'onconfirm={clear}', 'prompt(): Promise<void>']) assert.ok(!NATIVE_DIALOG.test(s), s);
});

test('the check itself catches each pattern', () => {
	for (const s of ["location.hash = '#/'", 'window.location.href = u', 'location.assign(u)']) {
		assert.ok(BANNED.some(([re]) => re.test(s)), s);
	}
	assert.ok(!BANNED.some(([re]) => re.test("if (location.hash === '#/') x()")));
});
