// Apps and plugins judged from their own top menus: nothing hard-coded per app.
import { suitability, type SBMenu } from '#lib/lms/slimbrowse';
import { ApiError } from '#lib/api/http';
import { TTL_MS, probeAll } from '#lib/lms/suitability';

test('menus with something to open, play or search are usable', () => {
	expect(suitability({ item_loop: [{ text: 'Search', type: 'search', actions: { go: { cmd: ['x', 'items'], params: { search: '__TAGGEDINPUT__' } } } }] })).toEqual({ state: 'ok' });
	expect(suitability({ item_loop: [{ text: 'Mood', actions: { go: { cmd: ['x', 'menu_mood'] } } }] })).toEqual({ state: 'ok' });
	const base: SBMenu = { base: { actions: { go: { cmd: ['x', 'items'], itemsParams: 'params' } } }, item_loop: [{ text: 'A', params: { item_id: '1' } }] };
	expect(suitability(base)).toEqual({ state: 'ok' });
});

test('a lone message means the app needs setting up in LMS', () => {
	// As SoundCloud says when no account is set.
	const r = suitability({ count: 1, item_loop: [{ text: 'Log in to SoundCloud in Advanced settings', type: 'text', style: 'itemNoAction' }] });
	expect(r).toEqual({ state: 'setup', message: 'Log in to SoundCloud in Advanced settings' });
	const long = suitability({ item_loop: [{ text: 'x'.repeat(1000), type: 'text' }] });
	expect(long.state === 'setup' && long.message.length).toBe(200);
});

test('empty menus, junk and errors are unusable', () => {
	// As Spotty answers when it isn't set up: count 0 and one blank item.
	expect(suitability({ count: 0, item_loop: [{ text: '' }] })).toEqual({ state: 'empty' });
	expect(suitability({})).toEqual({ state: 'empty' });
	expect(suitability({ item_loop: [null as never, 7 as never, 'x' as never] })).toEqual({ state: 'empty' });
	expect(suitability(null, new ApiError(403, 'not allowed'))).toEqual({ state: 'error', message: 'Not available remotely.' });
	expect(suitability(null, new Error('boom'))).toEqual({ state: 'error', message: 'The music server could not open it.' });
});

test('probes run a few at a time, and remembered answers are reused while fresh', async () => {
	const m = new Map<string, string>();
	const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
	let t = 1_000;
	let inFlight = 0;
	let peak = 0;
	const asked: string[] = [];
	const run = async (c: (string | number)[]) => {
		asked.push(String(c[0]));
		inFlight++;
		peak = Math.max(peak, inFlight);
		await new Promise((r) => setTimeout(r, 2));
		inFlight--;
		if (c[0] === 'broken') throw new Error('x');
		return c[0] === 'empty' ? { item_loop: [] } : { item_loop: [{ text: 'A', actions: { go: { cmd: ['a', 'items'] } } }] };
	};
	const targets = ['a', 'b', 'c', 'd', 'empty', 'broken'].map((k) => ({ key: k, command: [k, 'items', 0, 10] }));
	const got: Record<string, string> = {};
	await probeAll(targets, run, (k, s) => (got[k] = s.state), { store, concurrency: 2, now: () => t });
	expect(peak).toBeLessThanOrEqual(2);
	expect(got).toEqual({ a: 'ok', b: 'ok', c: 'ok', d: 'ok', empty: 'empty', broken: 'error' });
	asked.length = 0;
	t += TTL_MS - 1;
	await probeAll(targets, run, () => {}, { store, now: () => t });
	expect(asked).toEqual([]);
	t += 2;
	await probeAll(targets, run, () => {}, { store, now: () => t });
	expect(asked.length).toBe(6);
	// A tampered store is ignored.
	m.set('anywhere.apps.suitability', JSON.stringify({ a: { state: '<script>', at: t } }));
	asked.length = 0;
	await probeAll(targets.slice(0, 1), run, () => {}, { store, now: () => t });
	expect(asked).toEqual(['a']);
});
