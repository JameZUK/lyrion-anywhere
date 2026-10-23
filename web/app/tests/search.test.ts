// Library search: answers arriving out of order never replace the latest term's results, and
// the search is still there after going to a result and back.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import type { SearchResult } from '#lib/lms/types';
import { DEBOUNCE_MS, SearchState } from '#lib/search/store.svelte';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');

function later<T>() {
	let resolve!: (v: T) => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const albums = (...names: string[]): SearchResult => ({ albums_loop: names.map((album, i) => ({ album_id: i + 1, album })) }) as SearchResult;

describe('SearchState', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	function make() {
		const asked: { q: string; p: ReturnType<typeof later<SearchResult>> }[] = [];
		const s = new SearchState((q) => {
			const p = later<SearchResult>();
			asked.push({ q, p });
			return p.promise;
		});
		return { s, asked };
	}

	test('waits for a pause in typing, and needs two letters', async () => {
		const { s, asked } = make();
		s.set('a');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS * 2);
		expect(asked).toEqual([]);
		s.set('ab');
		s.set('abb');
		s.set('abba');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		expect(asked.map((a) => a.q)).toEqual(['abba']);
	});

	// Regression: a slow answer for an earlier term replaced the latest one's.
	test('a slow answer for an earlier term is dropped', async () => {
		const { s, asked } = make();
		s.set('ab');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		s.set('abba');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		asked[1]!.p.resolve(albums('Arrival'));
		await vi.advanceTimersByTimeAsync(0);
		asked[0]!.p.resolve(albums('Abbey Road'));
		await vi.advanceTimersByTimeAsync(0);
		expect(s.result?.albums_loop?.[0]?.album).toBe('Arrival');
		expect(s.shownFor).toBe('abba');
		expect(s.busy).toBe(false);
	});

	test('an answer arriving after the box was emptied is dropped, and the spinner stops', async () => {
		const { s, asked } = make();
		s.set('ab');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		expect(s.busy).toBe(true);
		s.set('');
		expect(s.busy).toBe(false);
		asked[0]!.p.resolve(albums('Abbey Road'));
		await vi.advanceTimersByTimeAsync(0);
		expect(s.result).toBeNull();
	});

	test('a stale failure is not shown; the latest one is', async () => {
		const { s, asked } = make();
		s.set('ab');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		s.set('abc');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		asked[0]!.p.reject(new Error('old'));
		await vi.advanceTimersByTimeAsync(0);
		expect(s.error).toBe('');
		asked[1]!.p.reject(new Error('Server busy'));
		await vi.advanceTimersByTimeAsync(0);
		expect(s.error).toBe('Server busy');
	});

	test('the same term again is not searched again; reset forgets', async () => {
		const { s, asked } = make();
		s.set('abba');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		asked[0]!.p.resolve(albums('Arrival'));
		await vi.advanceTimersByTimeAsync(0);
		s.set('abba ');
		await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
		expect(asked).toHaveLength(1);
		s.reset();
		expect(s.term).toBe('');
		expect(s.result).toBeNull();
	});
});

describe('search page', () => {
	test('keeps the term and results after leaving and coming back', async () => {
		fake.handler.rpc = async (_p, c) => (c[0] === 'search' ? { albums_loop: [{ album_id: 7, album: 'Arrival' }] } : {});
		const { default: SearchPage } = await import('../src/routes/search/+page.svelte');
		const first = render(SearchPage);
		const box = first.container.querySelector<HTMLInputElement>('[data-search-input]')!;
		await fireEvent.input(box, { target: { value: 'abba' } });
		await waitFor(() => expect(first.container.textContent).toContain('Arrival'));
		await waitFor(() => expect(first.container.querySelector('[role="status"]')!.textContent).toBe('1 result for “abba”.'));
		first.unmount();
		const again = render(SearchPage);
		expect(again.container.querySelector<HTMLInputElement>('[data-search-input]')!.value).toBe('abba');
		expect(again.container.textContent).toContain('Arrival');
		again.unmount();
	});
});
