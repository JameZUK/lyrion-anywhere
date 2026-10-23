// The version the web app shows is the plugin's (install.xml), the release it ships in. The
// app's own package.json version had been shown instead, so Settings › About said 0.1.0 for
// every release (the owner noticed after 0.5.1).
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render } from '@testing-library/svelte';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const { default: AboutSettings } = await import('#lib/settings/AboutSettings.svelte');

// Vitest runs from web/app (the config's root); the plugin tree is two levels up.
const installXml = readFileSync(resolve(process.cwd(), '../../plugin/Plugins/LyrionAnywhere/install.xml'), 'utf8');
const pluginVersion = installXml.match(/<version>\s*([^<\s]+)\s*<\/version>/)![1]!;

test('the build carries the plugin version from install.xml', () => {
	expect(__PLUGIN_VERSION__).toBe(pluginVersion);
	expect(__PLUGIN_VERSION__).toMatch(/^\d+\.\d+\.\d+$/);
	expect(__PLUGIN_VERSION__).not.toBe('0.1.0');
});

test('Settings › About shows it', () => {
	const r = render(AboutSettings);
	expect(r.container.querySelector('[data-app-version]')!.textContent).toMatch(new RegExp(`^Version ${pluginVersion.replace(/\./g, '\\.')} · build `));
	r.unmount();
});
