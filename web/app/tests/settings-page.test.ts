// Settings: what users change comes first; developer diagnostics come last, folded away.
import { render } from '@testing-library/svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');

beforeAll(() => {
	vi.stubGlobal('fetch', async () => new Response(JSON.stringify({}), { status: 200, headers: { 'content-type': 'application/json' } }));
	fake.app.players = [{ playerid: '02:00:00:00:00:01', name: 'Test player' }];
	fake.app.playerId = '02:00:00:00:00:01';
});
afterAll(() => vi.unstubAllGlobals());

test('user settings first; diagnostics last and collapsed', async () => {
	const { default: Settings } = await import('../src/routes/settings/+page.svelte');
	const r = render(Settings);
	const headings = [...r.container.querySelectorAll('.card > h2, details.developer > summary')].map((h) => h.textContent?.trim());
	expect(headings.slice(0, 8)).toEqual(['Appearance', 'Ratings', 'Lyrics', 'Visualiser', 'Haptics', 'Keyboard', 'Sleep timer', 'Streaming quality']);
	expect(headings.at(-2)).toBe('Diagnostics');
	expect(headings.at(-1)).toBe('About');
	const dev = r.container.querySelector<HTMLDetailsElement>('details[data-developer]')!;
	expect(dev.open).toBe(false);
	// The diagnostics are inside it, not in the user-facing cards.
	expect(dev.textContent).toContain('Playback capabilities');
	for (const card of r.container.querySelectorAll('.card:not(.developer)')) expect(card.textContent).not.toContain('Playback capabilities');
	r.unmount();
});

test('Settings › Listening history links to Your listening', async () => {
	const { default: Settings } = await import('../src/routes/settings/+page.svelte');
	const r = render(Settings);
	const card = [...r.container.querySelectorAll('.card')].find((c) => c.querySelector('h2')?.textContent === 'Listening history')!;
	const a = card.querySelector<HTMLAnchorElement>('a[data-stats-link]')!;
	expect(a.textContent).toBe('Your listening');
	expect(a.getAttribute('href')).toBe('#/stats');
	r.unmount();
});

test('Settings › Downloads: Wi-Fi only (on by default) and a storage limit (off by default)', async () => {
	localStorage.removeItem('anywhere.downloads');
	const { default: Settings } = await import('../src/routes/settings/+page.svelte');
	const r = render(Settings);
	const card = [...r.container.querySelectorAll('.card')].find((c) => c.querySelector('h2')?.textContent === 'Downloads')!;
	const wifi = [...card.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find((i) => i.parentElement?.textContent?.includes('Download on Wi-Fi only'))!;
	expect(wifi.checked).toBe(true);
	// jsdom, like Safari and Firefox, doesn't report the network type.
	expect(card.querySelector('[data-testid="wifi-only-note"]')!.textContent).toContain('This browser doesn’t say which network it is on, so downloads always start here');
	const cap = [...card.querySelectorAll<HTMLSelectElement>('select')].find((s) => s.parentElement?.textContent?.includes('Storage limit for downloads'))!;
	expect(cap.value).toBe('0');
	expect([...cap.options].map((o) => o.textContent)).toEqual(['No limit', '1.00 GB', '2.00 GB', '5.00 GB', '10.00 GB', '20.00 GB', '50.00 GB', '100.00 GB']);
	cap.value = String(5 * 1024 ** 3);
	cap.dispatchEvent(new Event('change', { bubbles: true }));
	wifi.click();
	expect(JSON.parse(localStorage.getItem('anywhere.downloads')!)).toMatchObject({ wifiOnly: false, capBytes: 5 * 1024 ** 3 });
	expect(card.querySelector('[data-testid="persist-status"]')!.textContent).toBeTruthy();
	r.unmount();
});
