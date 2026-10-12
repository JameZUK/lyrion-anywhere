// Lyrics (wave 1 §9): the gateway's answer checked as untrusted input, the current line by
// position, fetching once per track, and the view: the Lyrics button, the peek, tap to seek,
// and the setting that hides the peek.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { lineAt, loadPeek, lyricsPath, sanitiseLyrics, savePeek, MAX_LINES, MAX_LINE_CHARS } from '$lib/lyrics/lyrics';
import { LyricsState, REMEMBERED } from '$lib/lyrics/store.svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { lyrics } = await import('$lib/state/lyrics');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');

const SYNCED = {
	source: 'tags',
	synced: true,
	lines: [
		{ t: 1000, text: 'First line' },
		{ t: 12500, text: 'Second line' },
		{ t: 20000, text: '' },
	],
};

function memStore() {
	const m = new Map<string, string>();
	return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe('sanitiseLyrics', () => {
	test('accepts the gateway answer', () => {
		expect(sanitiseLyrics(SYNCED)).toEqual(SYNCED);
		expect(sanitiseLyrics({ source: 'plugin', synced: false, lines: [{ t: 0, text: 'a' }] })?.lines).toEqual([{ t: 0, text: 'a' }]);
	});

	test('refuses anything else', () => {
		for (const v of [
			null,
			'text',
			42,
			[],
			{},
			{ source: 'web', synced: true, lines: [{ t: 0, text: 'a' }] },
			{ source: 'tags', synced: 'yes', lines: [{ t: 0, text: 'a' }] },
			{ source: 'tags', synced: true, lines: 'a' },
			{ source: 'tags', synced: true, lines: [] },
			{ source: 'tags', synced: true, lines: [{ t: 0, text: '   ' }] },
			{ source: 'tags', synced: true, lines: [{ t: 0, text: 5 }, null, 'x'] },
		]) {
			expect(sanitiseLyrics(v)).toBeNull();
		}
	});

	test('cleans hostile lines', () => {
		const l = sanitiseLyrics({
			source: 'tags',
			synced: true,
			lines: [
				{ t: -5, text: 'negative' },
				{ t: 1.5, text: 'fraction' },
				{ t: Number.MAX_SAFE_INTEGER + 2, text: 'huge' },
				{ t: 50, text: 'x'.repeat(MAX_LINE_CHARS * 3) },
				{ t: 10, text: '<img src=x onerror=alert(1)>' },
				{ t: 5 },
			],
		})!;
		expect(l.lines.map((x) => x.t)).toEqual([0, 0, 0, 10, 50]);
		expect(l.lines.every((x) => x.text.length <= MAX_LINE_CHARS)).toBe(true);
		const many = sanitiseLyrics({ source: 'tags', synced: false, lines: Array.from({ length: MAX_LINES * 2 }, () => ({ t: 9, text: 'y' })) })!;
		expect(many.lines.length).toBe(MAX_LINES);
		expect(many.lines.every((x) => x.t === 0)).toBe(true);
	});
});

describe('lineAt', () => {
	const l = sanitiseLyrics(SYNCED)!;
	test.each([
		[0, -1],
		[999, -1],
		[1000, 0],
		[12499, 0],
		[12500, 1],
		[1e9, 2],
		[NaN, -1],
	])('%d ms → %d', (ms, want) => expect(lineAt(l, ms)).toBe(want));
	test('plain lyrics and none have no current line', () => {
		expect(lineAt({ ...l, synced: false }, 5000)).toBe(-1);
		expect(lineAt(null, 5000)).toBe(-1);
	});
});

test('lyricsPath takes positive integer ids only', () => {
	expect(lyricsPath(9)).toBe('/v1/lyrics/track/9');
	for (const v of [0, -1, 1.5, NaN, '9', '../x', null, undefined, Number.MAX_SAFE_INTEGER + 1]) expect(lyricsPath(v)).toBeNull();
});

test('the peek setting is on unless switched off', () => {
	const s = memStore();
	expect(loadPeek(s)).toBe(true);
	savePeek(false, s);
	expect(loadPeek(s)).toBe(false);
	savePeek(true, s);
	expect(loadPeek(s)).toBe(true);
	const denied = () => {
		throw new Error('denied');
	};
	const broken = { getItem: denied, setItem: denied };
	expect(loadPeek(broken)).toBe(true);
	expect(() => savePeek(false, broken)).not.toThrow();
});

describe('LyricsState', () => {
	afterEach(() => vi.unstubAllGlobals());

	function stubFetch(answer: (path: string) => { status: number; body: unknown }) {
		const paths: string[] = [];
		vi.stubGlobal('fetch', async (path: string, init: RequestInit) => {
			paths.push(path);
			expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok');
			const a = answer(path);
			return new Response(JSON.stringify(a.body), { status: a.status, headers: { 'content-type': 'application/json' } });
		});
		return paths;
	}
	const authed = <T,>(fn: (t: string) => Promise<T>) => fn('tok');

	test('fetches once per track and remembers "none"', async () => {
		const paths = stubFetch((p) => (p.endsWith('/9') ? { status: 200, body: SYNCED } : { status: 404, body: { error: 'no lyrics' } }));
		const s = new LyricsState(authed);
		await s.load(9);
		expect(s.lyrics?.lines[0]?.text).toBe('First line');
		await s.load(7);
		expect(s.lyrics).toBeNull();
		await s.load(9);
		await s.load(7);
		expect(s.lyrics).toBeNull();
		expect(paths).toEqual(['/v1/lyrics/track/9', '/v1/lyrics/track/7']);
	});

	test('failures other than 404 are tried again later', async () => {
		let status = 503;
		const paths = stubFetch(() => ({ status, body: status === 200 ? SYNCED : { error: 'busy' } }));
		const s = new LyricsState(authed);
		await s.load(9);
		expect(s.lyrics).toBeNull();
		await s.load(null);
		status = 200;
		await s.load(9);
		expect(s.lyrics?.synced).toBe(true);
		expect(paths.length).toBe(2);
	});

	test('bad ids are never fetched', async () => {
		const paths = stubFetch(() => ({ status: 200, body: SYNCED }));
		const s = new LyricsState(authed);
		for (const id of [0, -3, 1.5, '9', '../../x', null]) await s.load(id);
		expect(paths).toEqual([]);
		expect(s.lyrics).toBeNull();
	});

	test('a slow answer for an earlier track does not replace the current one', async () => {
		const resolvers: ((r: Response) => void)[] = [];
		vi.stubGlobal('fetch', () => new Promise<Response>((r) => resolvers.push(r)));
		const s = new LyricsState(authed);
		const first = s.load(9);
		const second = s.load(8);
		resolvers[1]!(new Response(JSON.stringify({ source: 'tags', synced: false, lines: [{ t: 0, text: 'eight' }] }), { status: 200 }));
		await second;
		resolvers[0]!(new Response(JSON.stringify(SYNCED), { status: 200 }));
		await first;
		expect(s.trackId).toBe(8);
		expect(s.lyrics?.lines[0]?.text).toBe('eight');
	});

	test('remembers a bounded number of tracks', async () => {
		const paths = stubFetch(() => ({ status: 404, body: {} }));
		const s = new LyricsState(authed);
		for (let i = 1; i <= REMEMBERED + 5; i++) await s.load(i);
		await s.load(1); // forgotten, so asked again
		await s.load(REMEMBERED + 5); // still remembered
		expect(paths.length).toBe(REMEMBERED + 6);
	});
});

describe('Now playing', () => {
	class NoObserver {
		observe() {}
		unobserve() {}
		disconnect() {}
	}
	beforeAll(() => {
		vi.stubGlobal('IntersectionObserver', NoObserver);
		vi.stubGlobal('ResizeObserver', NoObserver);
		HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
			this.setAttribute('open', '');
		};
		HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
			this.removeAttribute('open');
			this.dispatchEvent(new Event('close'));
		};
	});

	let answers: Record<string, { status: number; body: unknown }> = {};
	beforeEach(() => {
		lyrics.reset();
		lyrics.setPeek(true);
		fake.app.seeks.length = 0;
		fake.app.players = [{ playerid: '02:00:00:00:00:01', name: 'Test player' }];
		fake.app.playerId = '02:00:00:00:00:01';
		fake.app.elapsed = 13;
		answers = { '/v1/lyrics/track/9': { status: 200, body: SYNCED } };
		vi.stubGlobal('fetch', async (path: string) => {
			const a = answers[path] ?? { status: 404, body: { error: 'no lyrics' } };
			return new Response(JSON.stringify(a.body), { status: a.status });
		});
		fake.app.now = { mode: 'play', time: 13, duration: 240, playlist_loop: [{ id: 9, title: 'Song', artist: 'Band', 'playlist index': 0 }] };
	});
	afterAll(() => vi.unstubAllGlobals());

	test('synced lyrics: a peek with the current line, which opens the view; tapping a line seeks', async () => {
		const r = render(NowPlaying);
		await waitFor(() => expect(r.container.querySelector('[data-lyrics-peek]')).not.toBeNull());
		expect(r.container.querySelector('[data-lyrics-peek]')!.textContent).toBe('Second line');
		fake.app.elapsed = 2;
		await tick();
		expect(r.container.querySelector('[data-lyrics-peek]')!.textContent).toBe('First line');

		await fireEvent.click(r.container.querySelector('[data-lyrics-peek]')!);
		const view = document.querySelector('[data-lyrics-view]')!;
		expect(view).not.toBeNull();
		const lines = [...view.querySelectorAll<HTMLButtonElement>('button.line')];
		expect(lines.map((b) => b.textContent)).toEqual(['First line', 'Second line', '♪']);
		expect(lines[0]!.getAttribute('aria-current')).toBe('true');
		await fireEvent.click(lines[1]!);
		expect(fake.app.seeks).toEqual([12.5]);
		r.unmount();
	});

	test('no peek when switched off, but the Lyrics button stays', async () => {
		lyrics.setPeek(false);
		const r = render(NowPlaying);
		await waitFor(() => expect(r.container.querySelector('[data-lyrics-button]')).not.toBeNull());
		expect(r.container.querySelector('[data-lyrics-peek]')).toBeNull();
		r.unmount();
	});

	test('plain lyrics: no peek, and the view only scrolls', async () => {
		answers['/v1/lyrics/track/9'] = { status: 200, body: { source: 'plugin', synced: false, lines: [{ t: 0, text: 'Just words' }] } };
		const r = render(NowPlaying);
		await waitFor(() => expect(r.container.querySelector('[data-lyrics-button]')).not.toBeNull());
		expect(r.container.querySelector('[data-lyrics-peek]')).toBeNull();
		await fireEvent.click(r.container.querySelector('[data-lyrics-button]')!);
		const view = document.querySelector('[data-lyrics-view]')!;
		expect(view.querySelector('button.line')).toBeNull();
		expect(view.textContent).toContain('Just words');
		expect(view.textContent).toContain('Music & Artist Information');
		r.unmount();
	});

	test('no lyrics, a stream, or hostile text: nothing shown, nothing run', async () => {
		fake.app.now = { mode: 'play', time: 1, playlist_loop: [{ id: 7, title: 'None', 'playlist index': 0 }] };
		let r = render(NowPlaying);
		await tick();
		await new Promise((res) => setTimeout(res, 0));
		expect(r.container.querySelector('[data-lyrics-button]')).toBeNull();
		r.unmount();

		fake.app.now = { mode: 'play', time: 1, playlist_loop: [{ id: 9, title: 'Radio', remote: 1, 'playlist index': 0 }] };
		r = render(NowPlaying);
		await tick();
		expect(r.container.querySelector('[data-lyrics-button]')).toBeNull();
		r.unmount();

		answers['/v1/lyrics/track/9'] = { status: 200, body: { source: 'tags', synced: true, lines: [{ t: 0, text: '<img src=x onerror="globalThis.pwned=1">' }] } };
		lyrics.reset();
		fake.app.now = { mode: 'play', time: 1, playlist_loop: [{ id: 9, title: 'Song', 'playlist index': 0 }] };
		fake.app.elapsed = 1;
		r = render(NowPlaying);
		await waitFor(() => expect(r.container.querySelector('[data-lyrics-peek]')).not.toBeNull());
		expect(r.container.querySelector('img[src="x"]')).toBeNull();
		expect(r.container.querySelector('[data-lyrics-peek]')!.textContent).toContain('<img');
		expect((globalThis as Record<string, unknown>).pwned).toBeUndefined();
		r.unmount();
	});
});
