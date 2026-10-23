import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

/**
 * The version the app shows is the plugin's (`plugin/Plugins/LyrionAnywhere/install.xml`),
 * the one release carrying the web app, read when the build is configured. The web app's
 * own package.json version never changed and had been shown instead, so Settings › About
 * said 0.1.0 for every release. The file must be there: a build that cannot find it fails
 * rather than quietly showing the wrong number (the 0.5.3 release zip did exactly that,
 * because the Docker build stages copied only `web/app`). A deliberately standalone build
 * can set ANYWHERE_WEB_VERSION instead.
 */
function pluginVersion(): string {
	const given = process.env.ANYWHERE_WEB_VERSION;
	if (given && /^[0-9]+(\.[0-9]+){1,3}$/.test(given)) return given;
	const path = fileURLToPath(new URL('../../plugin/Plugins/LyrionAnywhere/install.xml', import.meta.url));
	let xml: string;
	try {
		xml = readFileSync(path, 'utf8');
	} catch (e) {
		throw new Error(`cannot read the plugin version from ${path} (${(e as Error).message}); build from the full checkout, or set ANYWHERE_WEB_VERSION`);
	}
	const m = xml.match(/<version>\s*([0-9]+(?:\.[0-9]+){1,3})\s*<\/version>/);
	if (!m) throw new Error(`no <version> in ${path}`);
	return m[1]!;
}

export default defineConfig({
	plugins: [
		// SvelteKit 3 takes its configuration here; it no longer reads svelte.config.js.
		sveltekit({
			preprocess: vitePreprocess(),
			// A single static page served by the gateway; all data comes from the /v1 API.
			adapter: adapter({ fallback: 'index.html', strict: true }),
			// Hash routes need no server-side fallback, so the gateway serves plain files only.
			router: { type: 'hash' },
			// Relative asset paths: the app works under any base path.
			paths: { relative: true },
			output: { bundleStrategy: 'split' },
		}),
		svelteTesting(),
	],
	define: {
		__PLUGIN_VERSION__: JSON.stringify(pluginVersion()),
	},
	server: {
		// Development: proxy the API to a local gateway (see README).
		proxy: {
			'/v1': { target: process.env.ANYWHERE_GATEWAY ?? 'http://127.0.0.1:9443', ws: true },
			'/healthz': process.env.ANYWHERE_GATEWAY ?? 'http://127.0.0.1:9443',
		},
	},
	test: {
		environment: 'jsdom',
		globals: true,
		include: ['tests/**/*.test.ts'],
		setupFiles: ['tests/setup.ts'],
	},
});
