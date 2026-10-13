// The ratings store: answers that arrive out of order never overwrite newer state, and one
// user's ratings are never shown to the next user to sign in on the same tab.
import type { RatingStyle, RoutineApi, TrackRating } from '$lib/api/routine';
import { RatingsState } from '$lib/ratings/store.svelte';
import { resetOnUserChange } from '$lib/state/per-user';
import type { Session } from '$lib/api/auth';

/** A promise the test settles by hand. */
function later<T>() {
	let resolve!: (v: T) => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

type Pending<T> = ReturnType<typeof later<T>>;

function fakeApi() {
	const lists: { tracks: number[]; p: Pending<TrackRating[]> }[] = [];
	const rates: { track: number; value: number; p: Pending<TrackRating | null> }[] = [];
	const styles: Pending<{ style: RatingStyle; count: number }>[] = [];
	const setStyles: { style: RatingStyle; p: Pending<unknown> }[] = [];
	const api = {
		ratings(tracks: number[]) {
			const p = later<TrackRating[]>();
			lists.push({ tracks, p });
			return p.promise;
		},
		rate(track: number, value: number) {
			const p = later<TrackRating | null>();
			rates.push({ track, value, p });
			return p.promise;
		},
		style() {
			const p = later<{ style: RatingStyle; count: number }>();
			styles.push(p);
			return p.promise;
		},
		setStyle(style: RatingStyle) {
			const p = later<unknown>();
			setStyles.push({ style, p });
			return p.promise;
		},
		async clearRatings() {
			return { removed: 2 };
		},
	};
	return { api: api as unknown as RoutineApi, lists, rates, styles, setStyles };
}

const r = (track: number, mine: number | null, shared: number | null = null): TrackRating => ({ track, mine, shared, rating: mine ?? shared });
const flush = () => new Promise((res) => setTimeout(res, 0));

async function ready() {
	const f = fakeApi();
	const s = new RatingsState(f.api);
	const first = s.load([]);
	f.styles[0]!.resolve({ style: 'stars', count: 0 });
	await first;
	return { s, ...f };
}

test('a slow fetch does not overwrite a rating the user has just given', async () => {
	const { s, lists, rates } = await ready();
	const loading = s.load([5]);
	await flush();
	const rating = s.rate(5, 80);
	expect(s.get(5)?.mine).toBe(80);
	// The fetch (asked before the change) answers late with the old value.
	lists[0]!.p.resolve([r(5, 20, 40)]);
	await loading;
	expect(s.get(5)?.mine).toBe(80);
	rates[0]!.p.resolve(r(5, 80, 40));
	await rating;
	expect(s.get(5)).toEqual(r(5, 80, 40));
});

test('two quick changes: the first answer arriving last does not win', async () => {
	const { s, rates } = await ready();
	const a = s.rate(5, 60);
	const b = s.rate(5, 100);
	rates[1]!.p.resolve(r(5, 100));
	await b;
	rates[0]!.p.resolve(r(5, 60));
	await a;
	expect(s.get(5)?.mine).toBe(100);
});

test('a refused change that was overtaken does not put the old value back', async () => {
	const { s, rates } = await ready();
	const a = s.rate(5, 60);
	const b = s.rate(5, 100);
	rates[0]!.p.reject(new Error('refused'));
	expect(await a).toBe(false);
	expect(s.get(5)?.mine).toBe(100);
	rates[1]!.p.resolve(r(5, 100));
	expect(await b).toBe(true);
	expect(s.get(5)?.mine).toBe(100);
});

test('the latest refused change is put back', async () => {
	const { s, rates } = await ready();
	const a = s.rate(5, 60);
	rates[0]!.p.reject(new Error('refused'));
	expect(await a).toBe(false);
	expect(s.get(5)).toBeNull();
});

test('a track being fetched is not asked for again, and only tracks asked about are kept', async () => {
	const { s, lists } = await ready();
	const one = s.load([5, 6]);
	const two = s.load([5, 6]);
	await two;
	expect(lists).toHaveLength(1);
	lists[0]!.p.resolve([r(5, 20), r(99, 100)]);
	await one;
	expect(s.get(5)?.mine).toBe(20);
	expect(s.get(99)).toBeNull();
});

test('answers asked for before a reset are dropped', async () => {
	const { s, lists, rates } = await ready();
	const loading = s.load([5]);
	await flush();
	const rating = s.rate(6, 80);
	s.reset();
	lists[0]!.p.resolve([r(5, 20)]);
	rates[0]!.p.resolve(r(6, 80));
	await loading;
	await rating;
	expect(s.get(5)).toBeNull();
	expect(s.get(6)).toBeNull();
});

test('a refused style change is put back and reported', async () => {
	const { s, setStyles } = await ready();
	const change = s.setStyle('heart');
	expect(s.style).toBe('heart');
	setStyles[0]!.p.reject(new Error('offline'));
	await expect(change).rejects.toThrow('offline');
	expect(s.style).toBe('stars');
});

test('a style fetch answering after a change does not undo it', async () => {
	const f = fakeApi();
	const s = new RatingsState(f.api);
	const loading = s.load([]);
	const change = s.setStyle('heart');
	f.setStyles[0]!.p.resolve({});
	await change;
	f.styles[0]!.resolve({ style: 'stars', count: 0 });
	await loading;
	expect(s.style).toBe('heart');
});

describe('a change of user resets per-user stores', () => {
	function fakeSession() {
		let fn: ((s: Session | null, st: string) => void) | null = null;
		const as = (username: string | null) => (username ? ({ user: { username, role: 'user' } } as unknown as Session) : null);
		return {
			subscribe(f: (s: Session | null, st: string) => void) {
				fn = f;
				f(null, 'unknown');
				return () => (fn = null);
			},
			emit(username: string | null) {
				fn?.(as(username), username ? 'signed-in' : 'signed-out');
			},
		};
	}

	test('signing out (and in as someone else) resets; renewals do not', () => {
		const session = fakeSession();
		const reset = vi.fn();
		resetOnUserChange(session, reset);
		expect(reset).not.toHaveBeenCalled(); // the first report
		session.emit('alice');
		expect(reset).toHaveBeenCalledTimes(1); // unknown → alice (nothing to show yet, harmless)
		session.emit('alice'); // a renewed token
		expect(reset).toHaveBeenCalledTimes(1);
		session.emit(null);
		expect(reset).toHaveBeenCalledTimes(2);
		session.emit('bob');
		expect(reset).toHaveBeenCalledTimes(3);
	});

	test("the app's ratings forget the previous user on sign-out", async () => {
		vi.resetModules();
		const session = fakeSession();
		const f = fakeApi();
		vi.doMock('$lib/state/app.svelte', () => ({ session }));
		vi.doMock('$lib/state/me', () => ({ routine: f.api }));
		const { ratings } = await import('$lib/state/ratings');
		session.emit('alice');
		const rating = ratings.rate(5, 80);
		f.rates[0]!.p.resolve(r(5, 80));
		await rating;
		expect(ratings.get(5)?.mine).toBe(80);
		session.emit(null);
		expect(ratings.get(5)).toBeNull();
		vi.doUnmock('$lib/state/app.svelte');
		vi.doUnmock('$lib/state/me');
	});
});
