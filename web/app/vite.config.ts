import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

/**
 * The version the app shows is the plugin's (`plugin/Plugins/LyrionAnywhere/install.xml`),
 * the one release carrying the web app, read when the build is configured. The web app's
 * own package.json version never changed and had been shown instead, so Settings › About
 * said 0.1.0 for every release. A standalone build without the plugin tree falls back to
 * the package version.
 */
function pluginVersion(): string {
	try {
		const xml = readFileSync(fileURLToPath(new URL('../../plugin/Plugins/LyrionAnywhere/install.xml', import.meta.url)), 'utf8');
		const m = xml.match(/<version>\s*([0-9]+(?:\.[0-9]+){1,3})\s*<\/version>/);
		if (m) return m[1]!;
	} catch {
		/* not in the full checkout */
	}
	return (JSON.parse(readFileSync(fileURLToPath(new URL('./package.json', import.meta.url)), 'utf8')) as { version: string }).version;
}

export default defineConfig({
	plugins: [sveltekit(), svelteTesting()],
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
