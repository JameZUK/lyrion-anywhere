// In-app navigation must go through SvelteKit's router. With the hash router, assigning
// `location.hash` (or `location.href`) makes SvelteKit reload the whole page, which stops this
// device's player mid-song. Use `go()` from `$lib/nav`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = new URL('../../src/', import.meta.url).pathname;
const BANNED = [
	[/\blocation\.(?:hash|href)\s*=(?!=)/, 'location.hash/href assignment'],
	[/\blocation\.assign\s*\(/, 'location.assign'],
];
// The service worker has no router.
const SKIP = ['service-worker.ts'];

function files(dir) {
	return readdirSync(dir, { recursive: true, withFileTypes: true })
		.filter((d) => d.isFile() && /\.(svelte|ts|js)$/.test(d.name) && !SKIP.includes(d.name))
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

test('the check itself catches each pattern', () => {
	for (const s of ["location.hash = '#/'", 'window.location.href = u', 'location.assign(u)']) {
		assert.ok(BANNED.some(([re]) => re.test(s)), s);
	}
	assert.ok(!BANNED.some(([re]) => re.test("if (location.hash === '#/') x()")));
});
