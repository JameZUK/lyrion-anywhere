// GET /v1/me/stats (wave3-design.md §1): what is sent, and strict parsing of the answer.
// The gateway is trusted but checked: malformed fields are clamped or left out, ids come only
// from the server, and names are plain text.
import { MeApi, STATS_LIMITS, parseStats, statsDate, statsName, statsPeriod, statsTz, type ListeningStats } from '#lib/api/me';
import { ApiError } from '#lib/api/http';

let calls: { url: string; method: string; headers: Record<string, string> }[] = [];
let reply: { status: number; body: unknown } = { status: 200, body: {} };

beforeEach(() => {
	calls = [];
	reply = { status: 200, body: {} };
	globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
		calls.push({ url: String(url), method: init?.method ?? 'GET', headers: init?.headers as Record<string, string> });
		return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
	}) as typeof fetch;
});

const me = new MeApi((fn) => fn('tok'));

const GOOD = {
	enabled: true,
	retention_days: 396,
	period: { from: 1790000000, to: 1792600000 },
	totals: { plays: 412, seconds: 82800, tracks: 301, albums: 64, artists: 41, days: 27 },
	top_artists: [{ id: 12, name: 'An Example Band', plays: 40, seconds: 9100 }],
	top_albums: [{ id: 7, plays: 22, seconds: 5200 }],
	top_tracks: [{ id: 9, plays: 6, seconds: 1300 }],
	new_artists: [{ id: 31, name: 'Another Example', first_played: 1791000000 }],
	by_hour: Array.from({ length: 24 }, (_, i) => i),
	by_weekday: [12, 30, 1, 2, 3, 4, 5],
	by_month: [{ month: '2026-10', plays: 412, seconds: 82800 }],
	longest_streak: { days: 9, from: '2026-10-02', to: '2026-10-10' },
	busiest_day: { date: '2026-10-04', seconds: 14400 },
};

test('the request: period and tz in the query, with the bearer token and the client header', async () => {
	reply.body = GOOD;
	const s = await me.stats('month-2026-10', 60);
	expect(calls[0]).toMatchObject({ url: '/v1/me/stats?period=month-2026-10&tz=60', method: 'GET' });
	expect(calls[0]!.headers.Authorization).toBe('Bearer tok');
	expect(calls[0]!.headers['X-Anywhere-Client']).toBe('1');
	expect(s.totals.plays).toBe(412);
	await me.stats('days-30', -300);
	expect(calls[1]!.url).toBe('/v1/me/stats?period=days-30&tz=-300');
	await me.stats('all');
	expect(calls[2]!.url).toBe('/v1/me/stats?period=all&tz=0');
});

test('a well-formed answer comes through whole', () => {
	const s = parseStats(GOOD);
	expect(s).toEqual({ ...GOOD });
});

test('bad periods and offsets are refused before anything is sent', async () => {
	const bad = [
		'', 'days-6', 'days-401', 'days-030', 'days-0', 'days--1', 'days-1e2', 'days-30 ', ' all', 'ALL', 'month-2026-13', 'month-2026-00', 'month-2026-1',
		'month-26-10', 'year-26', 'year-1969', 'year-20261', 'all&tz=1', 'all\n', 'days-30&period=all', '../history', 'year-2026/../x', 'everything',
	];
	for (const p of bad) await expect(me.stats(p, 0), p).rejects.toThrow(RangeError);
	for (const tz of [841, -841, 1.5, NaN, Infinity, '60' as unknown as number]) await expect(me.stats('all', tz), String(tz)).rejects.toThrow(RangeError);
	expect(calls).toHaveLength(0);
	for (const p of ['days-7', 'days-400', 'month-2026-01', 'month-1999-12', 'year-2026', 'all']) expect(statsPeriod(p)).toBe(p);
	expect(statsTz(840)).toBe(840);
	expect(statsTz(-840)).toBe(-840);
	expect(Object.is(statsTz(-0), 0)).toBe(true);
});

test('period parser: random strings never get through unless they are periods', () => {
	const alphabet = 'adlmnorsty-0123456789 %&=/.\\\u0000é';
	const valid = /^(days-(?:[7-9]|[1-9]\d|[1-3]\d\d|400)|month-(?:19[7-9]\d|[2-9]\d{3})-(?:0[1-9]|1[0-2])|year-(?:19[7-9]\d|[2-9]\d{3})|all)$/;
	let seed = 42;
	const rand = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
	for (let i = 0; i < 5000; i++) {
		const n = Math.floor(rand() * 16);
		let s = i % 3 === 0 ? ['days-', 'month-', 'year-', ''][Math.floor(rand() * 4)]! : '';
		for (let j = 0; j < n; j++) s += alphabet[Math.floor(rand() * alphabet.length)];
		let ok: boolean;
		try {
			statsPeriod(s);
			ok = true;
		} catch (e) {
			expect(e).toBeInstanceOf(RangeError);
			ok = false;
		}
		expect(ok, JSON.stringify(s)).toBe(valid.test(s));
	}
});

test('history switched off: zeros and empty lists, whatever else the answer says', () => {
	const s = parseStats({ ...GOOD, enabled: false });
	expect(s.enabled).toBe(false);
	expect(s.retention_days).toBe(396);
	expect(s.totals).toEqual({ plays: 0, seconds: 0, tracks: 0, albums: 0, artists: 0, days: 0 });
	expect(s.top_artists).toEqual([]);
	expect(s.by_hour).toHaveLength(24);
	expect(s.longest_streak).toBeNull();
	// Only `true` means on.
	for (const v of ['true', 1, null, undefined, {}]) expect(parseStats({ ...GOOD, enabled: v }).enabled).toBe(false);
});

test('hostile lists: bad ids, duplicates, extras and non-objects are left out; at most 20', () => {
	const s = parseStats({
		...GOOD,
		top_artists: [
			{ id: '12', name: 'String id', plays: 1, seconds: 1 },
			{ id: -3, name: 'Negative', plays: 1, seconds: 1 },
			{ id: 0, name: 'Zero', plays: 1, seconds: 1 },
			{ id: 1.5, name: 'Fraction', plays: 1, seconds: 1 },
			{ id: 2 ** 60, name: 'Huge', plays: 1, seconds: 1 },
			null,
			'x',
			[12],
			{ id: 5, name: 'Kept', plays: 3, seconds: 9 },
			{ id: 5, name: 'Duplicate', plays: 99, seconds: 99 },
		],
		top_albums: Array.from({ length: 50 }, (_, i) => ({ id: i + 1, plays: 1, seconds: 1 })),
		top_tracks: { id: 9 },
		new_artists: 'nope',
	});
	expect(s.top_artists).toEqual([{ id: 5, name: 'Kept', plays: 3, seconds: 9 }]);
	expect(s.top_albums).toHaveLength(STATS_LIMITS.list);
	expect(s.top_tracks).toEqual([]);
	expect(s.new_artists).toEqual([]);
});

test('hostile numbers are clamped: negative, fractional, NaN-ish, strings and huge values', () => {
	const s = parseStats({
		...GOOD,
		retention_days: -5,
		totals: { plays: -1, seconds: 12.9, tracks: '300', albums: 1e300, artists: null, days: Infinity },
		top_artists: [{ id: 1, name: 'A', plays: -10, seconds: 'lots' }],
		by_hour: [1, -2, 'x', null, 3.7, 1e300],
		by_weekday: Array(30).fill(2),
	});
	expect(s.retention_days).toBe(0);
	expect(s.totals).toEqual({ plays: 0, seconds: 12, tracks: 0, albums: 1e12, artists: 0, days: 0 });
	expect(s.top_artists[0]).toEqual({ id: 1, name: 'A', plays: 0, seconds: 0 });
	expect(s.by_hour).toHaveLength(24);
	expect(s.by_hour.slice(0, 7)).toEqual([1, 0, 0, 0, 3, 1e12, 0]);
	expect(s.by_weekday).toEqual(Array(7).fill(2));
});

test('names are plain text: markup stays literal, control and bidi characters go, length is bounded', () => {
	const s = parseStats({
		...GOOD,
		top_artists: [
			{ id: 1, name: '<img src=x onerror=alert(1)>', plays: 1, seconds: 1 },
			{ id: 2, name: 'Evil‮Reversed\u0000\u001b[31m', plays: 1, seconds: 1 },
			{ id: 3, name: 'x'.repeat(5000), plays: 1, seconds: 1 },
			{ id: 4, name: { toString: 'no' }, plays: 1, seconds: 1 },
		],
	});
	expect(s.top_artists[0]!.name).toBe('<img src=x onerror=alert(1)>');
	expect(s.top_artists[1]!.name).toBe('EvilReversed[31m');
	expect(s.top_artists[2]!.name.length).toBe(STATS_LIMITS.nameMax);
	expect(s.top_artists[3]!.name).toBe('');
	expect(statsName(42)).toBe('');
});

test('months, streaks and the busiest day must be real dates', () => {
	const s = parseStats({
		...GOOD,
		by_month: [
			{ month: '2026-13', plays: 1, seconds: 1 },
			{ month: '2026-02', plays: 2, seconds: 2 },
			{ month: '2026-01', plays: 1, seconds: 1 },
			{ month: '2026-02', plays: 9, seconds: 9 },
			{ month: '<b>', plays: 1, seconds: 1 },
		],
		longest_streak: { days: 9, from: '2026-02-30', to: '2026-03-02' },
		busiest_day: { date: '2026-10-04T00:00', seconds: 14400 },
	});
	expect(s.by_month.map((m) => m.month)).toEqual(['2026-01', '2026-02']);
	expect(s.by_month[1]!.plays).toBe(2);
	expect(s.longest_streak).toBeNull();
	expect(s.busiest_day).toBeNull();
	expect(parseStats({ ...GOOD, longest_streak: { days: 0, from: '2026-10-02', to: '2026-10-10' } }).longest_streak).toBeNull();
	expect(parseStats({ ...GOOD, longest_streak: { days: 3, from: '2026-10-10', to: '2026-10-02' } }).longest_streak).toBeNull();
	expect(parseStats({ ...GOOD, busiest_day: { date: '2026-10-04', seconds: 1e9 } }).busiest_day!.seconds).toBe(2 * 86400);
	expect(statsDate('2024-02-29')).toBe('2024-02-29');
	expect(statsDate('2026-02-29')).toBeNull();
	expect(parseStats({ ...GOOD, period: { from: 10, to: 5 } }).period).toBeNull();
});

test('an answer that is not an object is an error; any object parses without throwing', () => {
	for (const v of [null, 'x', 3, [], true]) expect(() => parseStats(v)).toThrow();
	let seed = 7;
	const rand = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
	const junk = (depth: number): unknown => {
		const k = Math.floor(rand() * 8);
		if (depth > 3 || k === 0) return [null, -1, 1.5, 'x', true, 1e308, 2 ** 53][Math.floor(rand() * 7)];
		if (k < 4) return Array.from({ length: Math.floor(rand() * 30) }, () => junk(depth + 1));
		const o: Record<string, unknown> = {};
		for (const key of ['id', 'name', 'plays', 'seconds', 'month', 'date', 'days', 'from', 'to', 'first_played']) if (rand() < 0.5) o[key] = junk(depth + 1);
		return o;
	};
	for (let i = 0; i < 500; i++) {
		const raw: Record<string, unknown> = { enabled: true };
		for (const key of Object.keys(GOOD)) if (key !== 'enabled' && rand() < 0.8) raw[key] = junk(0);
		const s: ListeningStats = parseStats(raw);
		expect(s.by_hour).toHaveLength(24);
		expect(s.by_weekday).toHaveLength(7);
		for (const l of [s.top_artists, s.top_albums, s.top_tracks, s.new_artists]) {
			expect(l.length).toBeLessThanOrEqual(20);
			for (const x of l) expect(Number.isSafeInteger(x.id) && x.id > 0).toBe(true);
		}
		for (const v of [...s.by_hour, ...s.by_weekday, ...Object.values(s.totals)]) expect(Number.isFinite(v) && v >= 0).toBe(true);
		for (const a of s.top_artists) expect(typeof a.name).toBe('string');
	}
});

test('gateway errors come through as ApiError', async () => {
	reply = { status: 400, body: { error: 'bad period' } };
	await expect(me.stats('all')).rejects.toBeInstanceOf(ApiError);
	reply = { status: 200, body: [1, 2] };
	await expect(me.stats('all')).rejects.toThrow('can’t read');
});
