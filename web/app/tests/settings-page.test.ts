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
	expect(headings.slice(0, 8)).toEqual(['Appearance', 'Ratings', 'Lyrics', 'Visualiser', 'Sleep timer', 'Streaming quality', 'Downloads', 'This device']);
	expect(headings.at(-2)).toBe('Diagnostics');
	expect(headings.at(-1)).toBe('About');
	const dev = r.container.querySelector<HTMLDetailsElement>('details[data-developer]')!;
	expect(dev.open).toBe(false);
	// The diagnostics are inside it, not in the user-facing cards.
	expect(dev.textContent).toContain('Playback capabilities');
	for (const card of r.container.querySelectorAll('.card:not(.developer)')) expect(card.textContent).not.toContain('Playback capabilities');
	r.unmount();
});
