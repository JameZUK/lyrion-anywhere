// Settings (Wave 3.5, rule 5): an index of rows with a one-line summary each, opening pages of
// their own; every setting the single long page had is on one of them (nothing removed); option
// lists open a picker; one switch component; diagnostics last, under About › Advanced, folded
// away; Account asks for the password once per action, in its own dialog.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string | undefined> = {};
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/settings') } }));
const fake = await import('./helpers/fake-app.svelte');
const { SECTIONS, sectionOf } = await import('$lib/settings/sections');

let account = { username: 'demo', role: 'admin', totp_enabled: false };
let meFeatures: Record<string, boolean> = { change_password: true };
const DEVICES = { devices: [{ id: 1, name: 'Test browser', current: true }, { id: 2, name: 'Phone', client_platform: 'Pixel 8', current: false }] };
const posts: { path: string; body: unknown }[] = [];
beforeAll(() => {
	vi.stubGlobal('fetch', async (path: string, init?: RequestInit) => {
		if (init?.method === 'POST') posts.push({ path, body: init.body ? JSON.parse(String(init.body)) : null });
		const p = String(path);
		const body = p.endsWith('/v1/account') ? account : p.endsWith('/v1/me') ? { features: meFeatures } : p.endsWith('/v1/devices') ? DEVICES : {};
		return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
	});
	fake.app.players = [{ playerid: '02:00:00:00:00:01', name: 'Test player' }];
	fake.app.playerId = '02:00:00:00:00:01';
	fake.session.current = { user: { username: 'demo', role: 'admin' }, device: { id: 1, name: 'Test browser' } };
});
afterAll(() => vi.unstubAllGlobals());

async function section(id: string) {
	params.section = id;
	const { default: Page } = await import('../src/routes/settings/[section]/+page.svelte');
	const r = render(Page);
	await tick();
	return r;
}

test('the index is seven or eight rows, each with a summary, linking to its page', async () => {
	const { default: Settings } = await import('../src/routes/settings/+page.svelte');
	const r = render(Settings);
	const rows = [...r.container.querySelectorAll<HTMLAnchorElement>('[data-settings-row]')];
	// Install shows only where the browser can install (not in jsdom).
	expect(rows.map((a) => a.querySelector('.label')!.textContent)).toEqual(['Account', 'Appearance', 'Playback', 'Now playing', 'Downloads', 'Listening history', 'About']);
	for (const a of rows) {
		expect(a.getAttribute('href')).toBe(`#/settings/${a.dataset.settingsRow}`);
		expect(a.querySelector('[data-summary]')!.textContent!.trim()).not.toBe('');
	}
	expect(rows[0]!.textContent).toContain('Signed in as demo');
	// Nothing else on the index: no settings laid out inline.
	expect(r.container.querySelectorAll('select, input, [role=switch]')).toHaveLength(0);
	r.unmount();
});

test('every section in an address opens its page; anything else is refused', async () => {
	for (const s of SECTIONS) expect(sectionOf(s.id)).toBe(s);
	for (const bad of ['', 'ACCOUNT', '../account', 'account/x', '__proto__', 'constructor', undefined, 7, null]) expect(sectionOf(bad)).toBeNull();
	const r = await section('nope');
	expect(r.container.textContent).toContain('Not a setting.');
	r.unmount();
});

test('Advanced is under About, and has the diagnostics last, folded away', async () => {
	let r = await section('about');
	expect(r.container.querySelector('a[data-advanced-link]')!.getAttribute('href')).toBe('#/settings/advanced');
	expect(r.getByRole('button', { name: /Keyboard shortcuts/ })).toBeTruthy();
	r.unmount();
	r = await section('advanced');
	expect(r.container.querySelector('h1')!.textContent).toBe('Advanced');
	const dev = r.container.querySelector<HTMLDetailsElement>('details[data-developer]')!;
	expect(dev.open).toBe(false);
	expect(dev.textContent).toContain('Playback capabilities');
	// The engine and the volume source are here, as pickers.
	expect(r.container.querySelector('[data-setting="playback-engine"]')).not.toBeNull();
	expect(r.container.querySelector('[data-setting="volume-source"]')).not.toBeNull();
	expect(r.container.querySelector('[data-player-start], [data-player-stop]')).not.toBeNull();
	r.unmount();
});

test('option lists open a picker; choosing one sets it (the sleep fade, in Playback)', async () => {
	const r = await section('playback');
	expect(r.container.querySelectorAll('select')).toHaveLength(0);
	const row = r.container.querySelector<HTMLButtonElement>('[data-setting="sleep-fade"]')!;
	expect(row.getAttribute('aria-haspopup')).toBe('dialog');
	await fireEvent.click(row);
	const picker = await waitFor(() => document.querySelector('[data-picker="sleep-fade"]')!);
	expect(picker.getAttribute('role')).toBe('radiogroup');
	await fireEvent.click(within(picker as HTMLElement).getByRole('radio', { name: '1 minute' }));
	expect(fake.app.sleepFade).toBe(60);
	expect(r.container.querySelector('[data-setting="start-with"]')).not.toBeNull();
	expect(r.container.querySelector('[data-setting="stream-quality"]')).not.toBeNull();
	expect(r.container.querySelector('[data-rename]')).not.toBeNull();
	r.unmount();
});

test('Now playing: the lyric line, the visualiser and haptics are the one switch; style and ratings are here too', async () => {
	const r = await section('now-playing');
	const switches = r.getAllByRole('switch').map((s) => s.getAttribute('aria-label'));
	expect(switches).toEqual(['Show the current lyric line', 'Visualiser', 'Haptics']);
	for (const s of r.getAllByRole('switch')) expect(s.textContent).toMatch(/\bO(n|ff)\b/);
	expect(r.container.querySelector('[data-setting="visualiser-style"]')).not.toBeNull();
	expect(r.container.querySelector('[data-rating-style]')).not.toBeNull();
	r.unmount();
});

test('Listening history links to Your listening', async () => {
	const r = await section('history');
	const a = r.container.querySelector<HTMLAnchorElement>('a[data-stats-link]')!;
	expect(a.textContent).toContain('Your listening');
	expect(a.getAttribute('href')).toBe('#/stats');
	r.unmount();
});

test('Downloads: Wi-Fi only (on by default) and a storage limit (off by default); Manage downloads links to the page', async () => {
	localStorage.removeItem('anywhere.downloads');
	const r = await section('downloads');
	const wifi = r.getByRole('switch', { name: 'Download on Wi-Fi only' });
	expect(wifi.getAttribute('aria-checked')).toBe('true');
	// jsdom, like Safari and Firefox, doesn't report the network type.
	expect(r.container.querySelector('[data-testid="wifi-only-note"]')!.textContent).toContain('This browser doesn’t say which network it is on, so downloads always start here');
	const cap = r.container.querySelector<HTMLButtonElement>('[data-setting="storage-limit"]')!;
	expect(cap.querySelector('[data-setting-value]')!.textContent).toBe('No limit');
	await fireEvent.click(cap);
	const picker = await waitFor(() => document.querySelector('[data-picker="storage-limit"]')!);
	expect([...picker.querySelectorAll('[role=radio]')].map((o) => o.textContent?.trim())).toEqual(['No limit', '1.00 GB', '2.00 GB', '5.00 GB', '10.00 GB', '20.00 GB', '50.00 GB', '100.00 GB']);
	await fireEvent.click(within(picker as HTMLElement).getByRole('radio', { name: '5.00 GB' }));
	await fireEvent.click(wifi);
	expect(JSON.parse(localStorage.getItem('anywhere.downloads')!)).toMatchObject({ wifiOnly: false, capBytes: 5 * 1024 ** 3 });
	expect(r.container.querySelector('[data-testid="persist-status"]')!.textContent).toBeTruthy();
	expect(r.container.querySelector('a[data-manage-downloads]')!.getAttribute('href')).toBe('#/downloads');
	r.unmount();
});

test('Account: one row per action, each asking for the password once, in its own dialog', async () => {
	const r = await section('account');
	await waitFor(() => expect(r.container.querySelector('[data-account-row="password"]')).not.toBeNull());
	// No password fields on the page itself.
	expect(r.container.querySelectorAll('input[type=password]')).toHaveLength(0);
	expect([...r.container.querySelectorAll('[data-account-row]')].map((b) => (b as HTMLElement).dataset.accountRow)).toEqual(['password', 'two-step', 'device']);
	await waitFor(() => expect(r.container.querySelector('[data-account-row="two-step"]')!.textContent).toContain('Off'));
	for (const row of ['password', 'two-step', 'device']) {
		await fireEvent.click(r.container.querySelector(`[data-account-row="${row}"]`)!);
		const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open]')!);
		expect(dialog.querySelectorAll('input[type=password][autocomplete="current-password"]').length, row).toBe(1);
		dialog.close();
		await waitFor(() => expect(document.querySelector('dialog[open]')).toBeNull());
	}
	// Sign out is there too.
	expect(r.getByRole('button', { name: /Sign out/ })).toBeTruthy();
	r.unmount();
});

test('Account › Your devices: only when the gateway offers it, in its own dialog', async () => {
	meFeatures = { change_password: true, devices: true };
	const r = await section('account');
	await waitFor(() => expect(r.container.querySelector('[data-account-row="devices"]')).not.toBeNull());
	expect([...r.container.querySelectorAll('[data-account-row]')].map((b) => (b as HTMLElement).dataset.accountRow)).toEqual(['password', 'two-step', 'devices', 'device']);
	await fireEvent.click(r.container.querySelector('[data-account-row="devices"]')!);
	const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open] [data-your-devices]')!);
	await waitFor(() => expect(dialog.querySelectorAll('[data-device]')).toHaveLength(2));
	expect(dialog.querySelector('[data-device="1"] [data-this-device]')).not.toBeNull();
	expect(dialog.querySelector('[data-sign-out-device="2"]')).not.toBeNull();
	// Nothing asks for the password here.
	expect(dialog.querySelectorAll('input[type=password]')).toHaveLength(0);
	document.querySelector<HTMLDialogElement>('dialog[open]')!.close();
	await waitFor(() => expect(document.querySelector('dialog[open]')).toBeNull());
	meFeatures = { change_password: true };
	r.unmount();
});

test('Account › Two-step verification: one name for the code everywhere ("Authenticator code")', async () => {
	account = { ...account, totp_enabled: true };
	const r = await section('account');
	await waitFor(() => expect(r.container.querySelector('[data-account-row="two-step"]')!.textContent).toContain('On'));
	await fireEvent.click(r.container.querySelector('[data-account-row="two-step"]')!);
	const dialog = await waitFor(() => document.querySelector<HTMLDialogElement>('dialog[open] [data-two-step]')!);
	expect(within(dialog as HTMLElement).getByLabelText('Authenticator code')).toBeTruthy();
	expect(within(dialog as HTMLElement).getByRole('button', { name: 'Turn off' })).toBeTruthy();
	account = { ...account, totp_enabled: false };
	r.unmount();
});

test('hostile values stay text on the index (a player name in the summary)', async () => {
	localStorage.setItem('anywhere.defaultPlayer.demo', JSON.stringify({ kind: 'player', id: '02:00:00:00:00:09' }));
	fake.app.players = [{ playerid: '02:00:00:00:00:09', name: '<img src=x onerror=alert(1)>' }];
	const { default: Settings } = await import('../src/routes/settings/+page.svelte');
	const r = render(Settings);
	expect(r.container.textContent).toContain('<img src=x onerror=alert(1)>');
	expect(r.container.querySelector('img[src="x"]')).toBeNull();
	localStorage.removeItem('anywhere.defaultPlayer.demo');
	fake.app.players = [{ playerid: '02:00:00:00:00:01', name: 'Test player' }];
	r.unmount();
});

test('every setting the one long page had is still reachable from the index', async () => {
	// What each page holds, by the hooks the old page had (and its sections).
	const where: Record<string, string[]> = {
		appearance: ['[role=radiogroup][aria-label="Theme"]'],
		playback: ['[data-setting="start-with"]', '[data-setting="stream-quality"]', '[data-setting="sleep-fade"]', '[data-rename]'],
		'now-playing': ['[data-lyrics-peek-setting]', '[data-visualiser-setting]', '[data-setting="visualiser-style"]', '[data-haptics-setting]', '[data-rating-style]', '[data-clear-ratings]'],
		downloads: ['[data-setting="format-wifi"]', '[data-setting="format-cellular"]', '[data-wifi-only]', '[data-setting="storage-limit"]', '[data-manage-downloads]'],
		about: ['[data-keys-help]', '[data-advanced-link]'],
		advanced: ['[data-setting="playback-engine"]', '[data-setting="volume-source"]', 'details[data-developer]'],
		history: ['[data-stats-link]'],
	};
	for (const [id, hooks] of Object.entries(where)) {
		const r = await section(id);
		for (const h of hooks) expect(r.container.querySelector(h), `${id}: ${h}`).not.toBeNull();
		r.unmount();
	}
	expect(screen).toBeTruthy();
});
