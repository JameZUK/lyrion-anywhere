// The web app manifest's shortcuts (wave2-design.md §5.5): Resume, Radio and Search, each a
// relative address inside the app's scope that opens a route the app has, with icons that
// exist. Nothing points off the app (no absolute or external addresses).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';

const root = new URL('../../', import.meta.url);
const manifest = JSON.parse(readFileSync(new URL('static/manifest.webmanifest', root), 'utf8'));

test('three shortcuts: Resume, Radio and Search', () => {
	assert.ok(Array.isArray(manifest.shortcuts));
	assert.deepEqual(
		manifest.shortcuts.map((s) => s.name),
		['Resume', 'Radio', 'Search'],
	);
	for (const s of manifest.shortcuts) {
		assert.equal(typeof s.short_name, 'string');
		assert.ok(s.short_name.length > 0 && s.short_name.length <= 12, s.short_name);
		assert.equal(typeof s.description, 'string');
	}
});

test('each opens a route the app has, inside its scope, with no query (the service worker serves it offline)', () => {
	const routes = {
		'/go/resume': 'src/routes/go/[action]/+page.svelte',
		'/go/radio': 'src/routes/go/[action]/+page.svelte',
		'/search': 'src/routes/search/+page.svelte',
	};
	const base = new URL('https://music.example.com/app/');
	const scope = new URL(manifest.scope, base);
	for (const s of manifest.shortcuts) {
		assert.match(s.url, /^\.\/#\//, s.url);
		const u = new URL(s.url, base);
		assert.equal(u.origin, base.origin);
		assert.ok(u.pathname.startsWith(scope.pathname), s.url);
		assert.equal(u.search, '');
		const route = u.hash.slice(1);
		assert.ok(route in routes, route);
		assert.ok(existsSync(new URL(routes[route], root)), routes[route]);
	}
	// The go route knows both actions.
	const go = readFileSync(new URL('src/routes/go/[action]/+page.svelte', root), 'utf8');
	assert.ok(go.includes("a !== 'resume' && a !== 'radio'"));
});

test('shortcut icons exist and are local', () => {
	for (const s of manifest.shortcuts) {
		assert.ok(Array.isArray(s.icons) && s.icons.length > 0);
		for (const i of s.icons) {
			assert.doesNotMatch(i.src, /^[a-z]+:|^\/\//i);
			assert.ok(existsSync(new URL(`static/${i.src}`, root)), i.src);
		}
	}
});

test('the manifest is still the installable one (name, start URL, scope, icons)', () => {
	assert.equal(manifest.start_url, './');
	assert.equal(manifest.scope, './');
	assert.equal(manifest.display, 'standalone');
	assert.ok(manifest.icons.some((i) => i.sizes === '512x512'));
});
