// Wave 2 daily routine on the web: ratings and alarms. The gateway's answers are checked as
// untrusted input, requests carry only checked values, and the views behave.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import {
	alarmBody,
	daysText,
	nextAlarm,
	parseTime,
	ratingValue,
	sanitiseAlarms,
	sanitiseRating,
	timeText,
	type Alarm,
} from '$lib/api/routine';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { ratings } = await import('$lib/state/ratings');
const { default: RatingControl } = await import('$lib/components/RatingControl.svelte');
const { default: AlarmsPage } = await import('../src/routes/alarms/+page.svelte');
const { default: RatingsSettings } = await import('$lib/components/RatingsSettings.svelte');
const { alarmChanges } = await import('$lib/rooms/alarm-changes.svelte');

const P = '02:00:00:00:00:01';

describe('checking answers', () => {
	test('ratings', () => {
		expect(sanitiseRating({ track: 1, mine: 80, shared: 60, rating: 75 })).toEqual({ track: 1, mine: 80, shared: 60, rating: 75 });
		expect(sanitiseRating({ track: 1, mine: 101, shared: -1, rating: 'x' })).toEqual({ track: 1, mine: null, shared: null, rating: null });
		for (const bad of [null, 'x', {}, { track: 0 }, { track: -1 }, { track: 1.5 }]) expect(sanitiseRating(bad)).toBeNull();
		expect(ratingValue(84)).toBe(80);
		expect(ratingValue(-5)).toBe(0);
		expect(ratingValue(1000)).toBe(100);
		expect(() => ratingValue(NaN)).toThrow();
	});

	test('alarms: bad entries dropped, nothing unexpected kept', () => {
		const l = sanitiseAlarms({
			enabled: false,
			alarms: [
				{ id: '1a2b3c4d', time: 25200, days: [5, 1, 1, 9], enabled: true, repeat: false, volume: 40, shuffle: 2, sound: { id: 'current', category: 'current', title: 'Current queue' } },
				{ id: 'XYZ', time: 1 },
				{ id: '1', time: 86400 },
				{ id: '2', time: 60, sound: { id: 'http://evil', title: 'x' }, url: 'file:///etc' },
			],
			sounds: [{ id: 'current', category: 'current', title: 'Current queue' }, { id: 'file:///x', title: 'bad' }, { id: '0123456789abcdef', category: 'favourites', title: 'Radio' }],
			id: '5e6f',
		});
		expect(l.enabled).toBe(false);
		expect(l.alarms.map((a) => a.id)).toEqual(['1a2b3c4d', '2']);
		expect(l.alarms[0]!.days).toEqual([1, 5]);
		expect(l.alarms[0]!.repeat).toBe(false);
		expect(l.alarms[1]!.sound?.id).toBeNull();
		expect(JSON.stringify(l)).not.toContain('file:');
		expect(l.sounds.map((s) => s.id)).toEqual(['current', '0123456789abcdef']);
		expect(l.id).toBe('5e6f');
		expect(sanitiseAlarms(null)).toEqual({ enabled: true, alarms: [], sounds: [] });
	});

	test('what is sent', () => {
		expect(alarmBody({ time: 0, days: [3, 1, 3], volume: 120.4, shuffle: 1, sound: 'current', enabled: true })).toEqual({
			time: 0,
			days: [1, 3],
			volume: 100,
			shuffle: 1,
			sound: 'current',
			enabled: true,
		});
		for (const bad of [{ time: 86400 }, { time: -1 }, { time: 1.5 }, { days: [] }, { days: [7] }, { sound: 'http://x' }, { sound: 'ABCDEF0123456789' }, { shuffle: 3 as 0 }]) {
			expect(() => alarmBody(bad)).toThrow();
		}
		// Unknown fields never reach the gateway.
		expect(alarmBody({ time: 5, url: 'file:///x' } as never)).toEqual({ time: 5 });
	});

	test('times and days', () => {
		expect(timeText(25200)).toBe('07:00');
		expect(timeText(86399)).toBe('23:59');
		expect(parseTime('7:05')).toBe(7 * 3600 + 300);
		for (const bad of ['', '24:00', '12:60', '7', '07:00:00', 'ab:cd']) expect(parseTime(bad)).toBeNull();
		expect(daysText([0, 1, 2, 3, 4, 5, 6])).toBe('Every day');
		expect(daysText([5, 4, 3, 2, 1])).toBe('Weekdays');
		expect(daysText([6, 0])).toBe('Weekends');
		expect(daysText([0, 1, 3])).toBe('Mon Wed Sun');
	});

	test('the next alarm', () => {
		const a = (time: number, days: number[], enabled = true) => ({ id: '1', time, days, enabled, repeat: true, volume: 50, shuffle: 0, sound: null }) as Alarm;
		const monday9 = new Date(2026, 9, 12, 9, 0, 0); // a Monday
		expect(nextAlarm([a(7 * 3600, [1])], monday9)?.getDate()).toBe(19); // next Monday
		expect(nextAlarm([a(10 * 3600, [1])], monday9)?.getHours()).toBe(10); // later today
		expect(nextAlarm([a(7 * 3600, [2]), a(8 * 3600, [2])], monday9)?.getHours()).toBe(7);
		expect(nextAlarm([a(7 * 3600, [2], false)], monday9)).toBeNull();
		expect(nextAlarm([], monday9)).toBeNull();
	});
});

// --- views ---------------------------------------------------------------------------

type Route = (method: string, path: string, body: unknown) => { status?: number; body: unknown };
let route: Route;
const sent: { method: string; path: string; body: unknown }[] = [];

beforeEach(() => {
	sent.length = 0;
	ratings.reset();
	vi.stubGlobal('fetch', async (path: string, init: RequestInit = {}) => {
		const method = init.method ?? 'GET';
		const body = init.body ? JSON.parse(String(init.body)) : undefined;
		sent.push({ method, path, body });
		const r = route(method, path, body);
		return new Response(JSON.stringify(r.body), { status: r.status ?? 200 });
	});
});
afterEach(() => vi.unstubAllGlobals());

describe('rating control', () => {
	test('stars: shows the blend, sets, clears, and takes half steps from the keyboard', async () => {
		let mine: number | null = null;
		route = (m, p, b) => {
			if (p === '/v1/me/ratings/style') return { body: { style: 'stars', count: 0 } };
			if (m === 'PUT') mine = (b as { rating: number }).rating || null;
			return { body: m === 'PUT' ? { track: 9, mine, shared: 60, rating: mine ?? 60 } : { ratings: [{ track: 9, mine, shared: 60, rating: 60 }] } };
		};
		const r = render(RatingControl, { track: 9 });
		const slider = await waitFor(() => {
			const s = r.container.querySelector<HTMLElement>('[data-rating-stars]');
			expect(s).not.toBeNull();
			return s!;
		});
		await waitFor(() => expect(slider.getAttribute('aria-valuetext')).toBe('Not rated by you; 3 of 5 on the server'));
		await fireEvent.click(r.container.querySelector('[data-star="4"]')!);
		await waitFor(() => expect(sent.at(-1)).toMatchObject({ method: 'PUT', path: '/v1/me/ratings/9', body: { rating: 80 } }));
		await waitFor(() => expect(slider.getAttribute('aria-valuenow')).toBe('80'));
		await fireEvent.keyDown(slider, { key: 'ArrowLeft' });
		await waitFor(() => expect(sent.at(-1)!.body).toEqual({ rating: 70 }));
		// Clicking your own rating's star clears it.
		await fireEvent.click(r.container.querySelector('[data-star="4"]')!);
		await waitFor(() => expect(sent.filter((s) => s.method === 'PUT').length).toBe(3));
	});

	test('a heart toggles between loved and not', async () => {
		route = (m, p, b) => {
			if (p === '/v1/me/ratings/style') return { body: { style: 'heart', count: 0 } };
			if (m === 'PUT') return { body: { track: 9, mine: (b as { rating: number }).rating || null, shared: null, rating: (b as { rating: number }).rating || null } };
			return { body: { ratings: [{ track: 9, mine: null, shared: null, rating: null }] } };
		};
		const r = render(RatingControl, { track: 9 });
		const heart = await waitFor(() => {
			const h = r.container.querySelector<HTMLButtonElement>('[data-rating-heart]');
			expect(h).not.toBeNull();
			return h!;
		});
		expect(heart.getAttribute('aria-pressed')).toBe('false');
		await fireEvent.click(heart);
		await waitFor(() => expect(heart.getAttribute('aria-pressed')).toBe('true'));
		expect(sent.at(-1)!.body).toEqual({ rating: 100 });
		await fireEvent.click(heart);
		await waitFor(() => expect(sent.at(-1)!.body).toEqual({ rating: 0 }));
	});

	test('a refused rating is put back and says so', async () => {
		route = (m, p) => (p === '/v1/me/ratings/style' ? { body: { style: 'stars' } } : m === 'PUT' ? { status: 503, body: { error: 'busy' } } : { body: { ratings: [] } });
		const r = render(RatingControl, { track: 9 });
		await waitFor(() => expect(r.container.querySelector('[data-star="5"]')).not.toBeNull());
		await fireEvent.click(r.container.querySelector('[data-star="5"]')!);
		await waitFor(() => expect(r.container.querySelector('[role="alert"]')?.textContent).toContain('Couldn’t save'));
		expect(r.container.querySelector('[data-rating-stars]')!.getAttribute('aria-valuenow')).toBe('0');
	});
});

describe('ratings settings', () => {
	test('clearing asks first, in the app; cancelling sends nothing', async () => {
		route = (m, p) => (p === '/v1/me/ratings/style' ? { body: { style: 'stars' } } : m === 'DELETE' ? { body: { removed: 4 } } : { body: {} });
		const native = vi.spyOn(window, 'confirm');
		const r = render(RatingsSettings);
		await fireEvent.click(r.container.querySelector('[data-clear-ratings]')!);
		const dialog = document.querySelector('[data-confirm-dialog]')!;
		expect(dialog.getAttribute('role')).toBe('alertdialog');
		await fireEvent.click(dialog.querySelector('[data-confirm-cancel]')!);
		expect(sent.some((s) => s.method === 'DELETE')).toBe(false);
		expect(document.querySelector('[data-confirm-dialog]')).toBeNull();
		await fireEvent.click(r.container.querySelector('[data-clear-ratings]')!);
		await fireEvent.click(document.querySelector('[data-confirm-ok]')!);
		await waitFor(() => expect(r.container.querySelector('[data-ratings-note]')!.textContent).toBe('4 ratings cleared.'));
		expect(sent.filter((s) => s.method === 'DELETE').map((s) => s.path)).toEqual(['/v1/me/ratings']);
		expect(native).not.toHaveBeenCalled();
	});

	test('a refused style change is put back and says why', async () => {
		route = (m, p) => (p === '/v1/me/ratings/style' && m === 'PUT' ? { status: 503, body: { error: 'busy' } } : { body: { style: 'stars' } });
		const r = render(RatingsSettings);
		const select = r.container.querySelector<HTMLSelectElement>('[data-rating-style]')!;
		await waitFor(() => expect(sent.some((s) => s.path === '/v1/me/ratings/style')).toBe(true));
		await fireEvent.change(select, { target: { value: 'heart' } });
		await waitFor(() => expect(r.container.querySelector('[data-ratings-note]')!.textContent).toContain('Couldn’t change how you rate'));
		expect(ratings.style).toBe('stars');
	});
});

describe('alarms page', () => {
	const listing = (alarms: unknown[] = [], enabled = true) => ({
		enabled,
		alarms,
		sounds: [
			{ id: 'current', category: 'current', title: 'Current queue' },
			{ id: '0123456789abcdef', category: 'favourites', title: 'Morning radio' },
		],
	});

	beforeEach(() => {
		fake.app.players = [{ playerid: P, name: 'Kitchen' }];
		fake.app.playerId = P;
	});

	test('lists alarms and adds one with checked values', async () => {
		const alarm = { id: '1a2b3c4d', time: 25200, days: [1, 2, 3, 4, 5], enabled: true, repeat: true, volume: 40, shuffle: 0, sound: { id: '0123456789abcdef', category: 'favourites', title: 'Morning radio' } };
		route = (m) => ({ body: m === 'POST' ? { ...listing([alarm]), id: '5e6f7a8b' } : listing([alarm]) });
		const r = render(AlarmsPage);
		await waitFor(() => expect(r.container.querySelector('[data-alarm="1a2b3c4d"]')).not.toBeNull());
		expect(r.container.textContent).toContain('07:00');
		expect(r.container.textContent).toContain('Weekdays · Morning radio');
		expect(r.container.querySelector('[data-next-alarm]')).not.toBeNull();
		await fireEvent.click(r.container.querySelector('[data-add-alarm]')!);
		const time = r.container.querySelector<HTMLInputElement>('[data-alarm-time]')!;
		await fireEvent.input(time, { target: { value: '06:30' } });
		const sound = r.container.querySelector<HTMLSelectElement>('[data-alarm-sound]')!;
		await fireEvent.change(sound, { target: { value: '0123456789abcdef' } });
		await fireEvent.submit(r.container.querySelector('[data-alarm-form]')!);
		await waitFor(() => expect(sent.some((s) => s.method === 'POST')).toBe(true));
		const post = sent.find((s) => s.method === 'POST')!;
		expect(post.path).toBe(`/v1/players/${P}/alarms`);
		expect(post.body).toEqual({ time: 6 * 3600 + 1800, days: [1, 2, 3, 4, 5], repeat: true, volume: 50, shuffle: 0, sound: '0123456789abcdef', enabled: true });
	});

	test('deleting asks first, in the page; the all switch', async () => {
		const alarm = { id: '1a2b3c4d', time: 3600, days: [0], enabled: true, repeat: true, volume: null, shuffle: 0, sound: null };
		route = (m) => ({ body: m === 'DELETE' ? listing([]) : listing([alarm]) });
		const r = render(AlarmsPage);
		await waitFor(() => expect(r.container.querySelector('[data-alarm="1a2b3c4d"]')).not.toBeNull());
		// Reading them changes nothing; a change tells Rooms to read the next alarms again.
		const before = alarmChanges.version;
		await fireEvent.click(r.container.querySelector('[aria-label="Delete the 01:00 alarm"]')!);
		expect(alarmChanges.version).toBe(before);
		expect(sent.some((s) => s.method === 'DELETE')).toBe(false);
		await fireEvent.click(r.container.querySelector('[data-confirm-delete]')!);
		await waitFor(() => expect(sent.some((s) => s.method === 'DELETE' && s.path === `/v1/players/${P}/alarms/1a2b3c4d`)).toBe(true));
		await waitFor(() => expect(alarmChanges.version).toBe(before + 1));
		await fireEvent.click(r.container.querySelector('[data-all-alarms]')!);
		await waitFor(() => expect(sent.at(-1)).toMatchObject({ method: 'PUT', path: `/v1/players/${P}/alarms/enabled`, body: { enabled: false } }));
	});

	test('no player chosen: nothing is asked', async () => {
		fake.app.playerId = '';
		route = () => ({ body: listing() });
		const r = render(AlarmsPage);
		expect(r.container.textContent).toContain('No player chosen');
		await new Promise((res) => setTimeout(res, 0));
		expect(sent).toEqual([]);
	});

	// Regression: a slow listing for the previous player replaced the new player's.
	test('switching players mid-request never shows the old player’s alarms', async () => {
		const OTHER = '02:00:00:00:00:02';
		fake.app.players = [
			{ playerid: P, name: 'Kitchen' },
			{ playerid: OTHER, name: 'Bedroom' },
		];
		const waiting: { path: string; answer: (body: unknown) => void }[] = [];
		vi.stubGlobal('fetch', (path: string) => {
			sent.push({ method: 'GET', path, body: undefined });
			return new Promise<Response>((res) => waiting.push({ path, answer: (b) => res(new Response(JSON.stringify(b), { status: 200 })) }));
		});
		const alarm = (id: string, time: number) => ({ id, time, days: [1], enabled: true, repeat: true, volume: null, shuffle: 0, sound: null });
		const r = render(AlarmsPage);
		await waitFor(() => expect(waiting).toHaveLength(1));
		fake.app.playerId = OTHER;
		await waitFor(() => expect(waiting).toHaveLength(2));
		expect(waiting[1]!.path).toBe(`/v1/players/${OTHER}/alarms`);
		waiting[1]!.answer(listing([alarm('bbbbbbbb', 7200)]));
		await waitFor(() => expect(r.container.querySelector('[data-alarm="bbbbbbbb"]')).not.toBeNull());
		// The first player's answer arrives late: ignored.
		waiting[0]!.answer(listing([alarm('aaaaaaaa', 3600)]));
		await new Promise((res) => setTimeout(res, 10));
		expect(r.container.querySelector('[data-alarm="aaaaaaaa"]')).toBeNull();
		expect(r.container.querySelector('[data-alarm="bbbbbbbb"]')).not.toBeNull();
		expect(r.container.textContent).toContain('On Bedroom');
	});

	test('a refusal is shown', async () => {
		route = () => ({ status: 403, body: { error: 'you may not control this player' } });
		const r = render(AlarmsPage);
		await waitFor(() => expect(r.container.querySelector('[role="alert"]')?.textContent).toContain('you may not control this player'));
	});
});
