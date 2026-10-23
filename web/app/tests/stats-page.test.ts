// Your listening: the page's states (loading, off, empty, data), names by id, the charts'
// text alternatives, and the link from Settings › Listening history.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { emptyStats, parseStats, type ListeningStats } from '#lib/api/me';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const state = {
	answer: null as null | ListeningStats,
	fail: '',
	pending: false,
	held: null as null | Map<string, (s: ListeningStats) => void>,
	asked: [] as { period: string; tz: number }[],
};
vi.mock('#lib/state/me', () => ({
	me: {
		async stats(period: string, tz: number) {
			state.asked.push({ period, tz });
			if (state.pending) return new Promise(() => {});
			if (state.held) return new Promise((resolve) => state.held!.set(period, resolve));
			if (state.fail) throw new Error(state.fail);
			return structuredClone(state.answer);
		},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { default: Stats } = await import('../src/routes/stats/+page.svelte');

function full(): ListeningStats {
	return parseStats({
		enabled: true,
		retention_days: 396,
		period: { from: 1790000000, to: 1792600000 },
		totals: { plays: 412, seconds: 82800, tracks: 301, albums: 64, artists: 41, days: 27 },
		top_artists: Array.from({ length: 14 }, (_, i) => ({ id: 100 + i, name: i === 0 ? '<b>An Example Band</b>' : `Example Artist ${i}`, plays: 40 - i, seconds: 9100 })),
		top_albums: [
			{ id: 7, plays: 22, seconds: 5200 },
			{ id: 8, plays: 10, seconds: 2000 },
		],
		top_tracks: [
			{ id: 9, plays: 6, seconds: 1300 },
			{ id: 10, plays: 5, seconds: 1000 },
		],
		new_artists: [{ id: 31, name: 'Another Example', first_played: 1791000000 }],
		by_hour: Array.from({ length: 24 }, (_, h) => (h === 21 ? 40 : h === 8 ? 10 : 0)),
		by_weekday: [12, 30, 1, 2, 3, 4, 5],
		by_month: [],
		longest_streak: { days: 9, from: '2026-10-02', to: '2026-10-10' },
		busiest_day: { date: '2026-10-04', seconds: 14400 },
	});
}

beforeEach(() => {
	vi.useFakeTimers({ toFake: ['Date'] });
	vi.setSystemTime(new Date(2026, 9, 13, 12, 0));
	state.answer = full();
	state.fail = '';
	state.pending = false;
	state.held = null;
	state.asked = [];
	fake.calls.length = 0;
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'albums') return { albums_loop: [{ id: 8, album: 'Second Example', artist: 'Someone' }, { id: 7, album: 'Blue Example', artist: 'An Example Band' }] };
		if (cmd[0] === 'titles') return { titles_loop: [{ id: 9, title: 'First Example Song', artist: 'An Example Band', album_id: 7 }] };
		return {};
	};
});
afterEach(() => vi.useRealTimers());

test('loading, then this month’s totals, asked with the device’s offset east of UTC', async () => {
	render(Stats);
	expect(screen.getByRole('status').textContent).toBe('Loading…');
	await waitFor(() => expect(screen.getByText('23 h')).toBeTruthy());
	expect(state.asked[0]).toEqual({ period: 'month-2026-10', tz: -new Date().getTimezoneOffset() || 0 });
	const dl = document.querySelector('dl.totals') as HTMLElement;
	expect(dl.textContent).toContain('Plays412');
	expect(dl.textContent).toContain('Artists41');
	expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Your listening');
});

test('switched off: says so and links to the setting', async () => {
	state.answer = emptyStats(false, 396);
	render(Stats);
	await waitFor(() => expect(document.querySelector('[data-stats-off]')).not.toBeNull());
	expect(document.body.textContent).toContain('Listening history is switched off');
	expect(screen.getByRole('link', { name: 'Listening history settings' }).getAttribute('href')).toBe('#/settings');
	expect(document.querySelector('[data-chart]')).toBeNull();
});

test('nothing yet: says how long until there is something to show', async () => {
	state.answer = emptyStats(true, 396);
	render(Stats);
	await waitFor(() => expect(document.querySelector('[data-stats-empty]')).not.toBeNull());
	expect(document.body.textContent).toContain('Nothing played this month yet.');
	expect(document.body.textContent).toContain('within five minutes');
});

test('an error says why and can be tried again', async () => {
	state.fail = 'Too many requests';
	render(Stats);
	await waitFor(() => expect(document.body.textContent).toContain('Too many requests'));
	state.fail = '';
	await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
	await waitFor(() => expect(screen.getByText('23 h')).toBeTruthy());
});

test('most played: artists with their names as text, albums and tracks by id, each linked', async () => {
	render(Stats);
	const section = (await screen.findByRole('heading', { name: 'Most played artists' })).closest('section')!;
	const items = within(section).getAllByRole('listitem');
	expect(items).toHaveLength(10);
	// Markup in a name is shown as text, never parsed.
	expect(items[0]!.textContent).toContain('<b>An Example Band</b>');
	expect(section.querySelector('b')).toBeNull();
	expect(within(items[0]!).getByRole('link').getAttribute('href')).toBe('#/artist/100');
	await fireEvent.click(within(section).getByRole('button', { name: 'Show 20' }));
	expect(within(section).getAllByRole('listitem')).toHaveLength(14);

	const albums = screen.getByRole('heading', { name: 'Most played albums' }).closest('section')!;
	await waitFor(() => expect(albums.textContent).toContain('Blue Example'));
	// In the order of the stats, not LMS's.
	expect(within(albums).getAllByRole('link').map((a) => a.getAttribute('href'))).toEqual(['#/album/7', '#/album/8']);
	expect(albums.textContent).toContain('22 plays · 1.4 h');
	const albumCall = fake.calls.find((c) => c.cmd[0] === 'albums')!;
	expect(albumCall.cmd.at(-1)).toBe('album_id:7,8');

	const tracks = screen.getByRole('heading', { name: 'Most played tracks' }).closest('section')!;
	await waitFor(() => expect(tracks.textContent).toContain('First Example Song'));
	expect(within(tracks).getByRole('link').getAttribute('href')).toBe('#/album/7');
	// A track LMS no longer knows: still listed, not linked.
	expect(tracks.textContent).toContain('A track no longer in the library');
	expect(fake.calls.find((c) => c.cmd[0] === 'titles')!.cmd.at(-1)).toBe('track_id:9,10');
});

test('charts have text alternatives: a summary sentence and a table of every value', async () => {
	render(Stats);
	await waitFor(() => expect(document.querySelectorAll('[data-chart]')).toHaveLength(2));
	const [hours, days] = [...document.querySelectorAll('[data-chart]')] as HTMLElement[];
	// The drawing itself is hidden from screen readers.
	expect(hours!.querySelector('svg')!.getAttribute('aria-hidden')).toBe('true');
	expect(hours!.querySelector('figcaption')!.textContent).toBe('You listen most between 21:00–22:00, with 40 plays.');
	const rows = within(hours!).getAllByRole('row', { hidden: true });
	expect(rows).toHaveLength(25);
	expect(rows[22]!.textContent).toBe('21:00–22:00' + '40 plays');
	expect(days!.querySelector('figcaption')!.textContent).toBe('Your busiest day of the week is Tuesday, with 30 plays.');
	expect(within(days!).getByRole('table', { hidden: true }).querySelector('caption')!.textContent).toBe('By day of the week');
	// No month chart for a month.
	expect(document.body.textContent).not.toContain('By month');
});

test('a year: the month chart, "first played since", and Your year in sound on demand', async () => {
	const s = full();
	s.by_month = [
		{ month: '2025-01', plays: 10, seconds: 3600 },
		{ month: '2025-02', plays: 50, seconds: 36000 },
	];
	state.answer = s;
	render(Stats);
	await waitFor(() => expect(screen.getByText('23 h')).toBeTruthy());
	await fireEvent.change(document.querySelector('[data-period]')!, { target: { value: 'year-2025' } });
	await waitFor(() => expect(state.asked.at(-1)!.period).toBe('year-2025'));
	await waitFor(() => expect(document.querySelectorAll('[data-chart]')).toHaveLength(3));
	const month = document.querySelectorAll('[data-chart]')[2]!;
	expect(month.querySelector('figcaption')!.textContent).toBe('Your biggest month was February 2025, with 10 hours and 50 plays.');
	expect(screen.getByRole('heading', { name: 'New artists in 2025' })).toBeTruthy();
	expect(document.body.textContent).toContain('first played since your history began');
	expect(screen.getByRole('button', { name: 'Play Your year in sound 2025' })).toBeTruthy();
});

test('streaks and the busiest day', async () => {
	render(Stats);
	const section = (await screen.findByRole('heading', { name: 'Streaks' })).closest('section')!;
	expect(section.textContent).toContain('9 days');
	expect(section.textContent).toContain('2 October 2026 to 10 October 2026');
	expect(section.textContent).toContain('4 October 2026');
	expect(section.textContent).toContain('4 hours');
});

test('the period picker offers the contract’s periods, and a slow old answer never replaces a newer one', async () => {
	render(Stats);
	await waitFor(() => expect(screen.getByText('23 h')).toBeTruthy());
	const options = [...(document.querySelector('[data-period]') as HTMLSelectElement).options].map((o) => o.value);
	expect(options).toEqual(['month-2026-10', 'days-30', 'year-2026', 'year-2025', 'all']);
	state.held = new Map();
	await fireEvent.change(document.querySelector('[data-period]')!, { target: { value: 'days-30' } });
	await fireEvent.change(document.querySelector('[data-period]')!, { target: { value: 'all' } });
	await waitFor(() => expect([...state.held!.keys()]).toEqual(['days-30', 'all']));
	const newer = full();
	newer.totals.seconds = 7200;
	state.held.get('all')!(newer);
	await waitFor(() => expect(screen.getByText('2 h')).toBeTruthy());
	const older = full();
	older.totals.seconds = 360000;
	state.held.get('days-30')!(older);
	await new Promise((r) => setTimeout(r, 20));
	expect(screen.queryByText('100 h')).toBeNull();
	expect(screen.getByText('2 h')).toBeTruthy();
});
