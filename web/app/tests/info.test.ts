// Artist biographies and photos, and album reviews (X5 part 1): the checks on the gateway's
// answers (hostile text stays text, photos only as the gateway's own image-proxy paths), the
// API call, the "Read more" disclosure, and the artist and album pages (lazy, quiet on failure).
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { Api } from '$lib/api/rpc';
import { ApiError } from '$lib/api/http';
import type { SessionManager } from '$lib/api/session';
import { MAX_TEXT, infoPath, isLong, paragraphs, photoPath, plainText, sanitiseAlbumInfo, sanitiseArtistInfo } from '$lib/info/info';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string | undefined> = {};
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/') } }));
vi.mock('$lib/offline/index.svelte', () => ({
	downloads: { savedById: () => undefined, album: () => null, items: [], item: () => undefined, online: true, savedAlbum: () => [] },
}));
const fake = await import('./helpers/fake-app.svelte');
const InfoText = (await import('$lib/components/InfoText.svelte')).default;
const ArtistPage = (await import('../src/routes/artist/[id]/[[role]]/+page.svelte')).default;
const AlbumPage = (await import('../src/routes/album/[id]/+page.svelte')).default;

const PHOTO = '/v1/art/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image';
const HOSTILE = '<img src=x onerror=alert(1)><script>alert(2)</script>';
const LONG = `First paragraph about the band, ${'and more words '.repeat(30)}end.\n\nSecond paragraph.\nWith a second line.`;

const settle = async () => {
	for (let i = 0; i < 6; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

beforeAll(() => {
	globalThis.IntersectionObserver = class {
		observe() {}
		disconnect() {}
		unobserve() {}
		takeRecords() {
			return [];
		}
		root = null;
		rootMargin = '';
		thresholds = [];
	} as unknown as typeof IntersectionObserver;
});

const realArt = fake.api.art;
beforeEach(() => {
	fake.calls.length = 0;
	fake.infoCalls.length = 0;
	fake.handler.rpc = async () => ({});
	fake.handler.info = async () => ({});
	fake.api.art = realArt;
	for (const k of Object.keys(params)) delete params[k];
});
afterEach(() => vi.unstubAllGlobals());

// --- checks -----------------------------------------------------------------------------------

describe('checks', () => {
	test('the gateway answers are accepted', () => {
		expect(sanitiseArtistInfo({ biography: 'One.\n\nTwo.', photo: PHOTO, credits: 'Discogs' })).toEqual({
			biography: 'One.\n\nTwo.',
			photo: 'imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image_600x600',
			credits: 'Discogs',
		});
		expect(sanitiseArtistInfo({ biography: null, photo: null, credits: null })).toEqual({ biography: null, photo: null, credits: null });
		expect(sanitiseAlbumInfo({ review: 'Fine.' })).toEqual({ review: 'Fine.' });
		expect(sanitiseAlbumInfo({ review: null })).toEqual({ review: null });
		expect(photoPath(PHOTO, 5000)).toMatch(/image_1000x1000$/);
	});

	test('anything else is refused', () => {
		for (const v of [null, undefined, 'x', 5, [], [{ biography: 'x' }]]) {
			expect(sanitiseArtistInfo(v)).toEqual({ biography: null, photo: null, credits: null });
			expect(sanitiseAlbumInfo(v)).toEqual({ review: null });
		}
		for (const photo of [
			'https://images.example.com/a.jpg',
			'imageproxy/https%3A%2F%2Fimages.example.com%2Fa.jpg/image',
			'/v1/art/music/1/cover',
			'/v1/art/html/images/x.png',
			'/v1/me',
			'/v1/art/imageproxy/https%3A%2F%2Fx.example/image/../../../v1/me',
			'/v1/art/imageproxy/file%3A%2F%2F%2Fetc%2Fpasswd/image',
			'/v1/art/imageproxy/https%3A%2F%2Fx.example/image?x=1',
			'/v1/art/imageproxy/https%3A%2F%2Fx y.example/image',
			'/v1/art/imageproxy/javascript%3Aalert(1)/image',
			'//evil.example/v1/art/imageproxy/https%3A%2F%2Fx.example/image',
			{ toString: () => PHOTO },
		]) {
			const a = sanitiseArtistInfo({ photo, credits: 'Discogs' });
			expect(a.photo, String(photo)).toBeNull();
			expect(a.credits).toBeNull();
		}
	});

	test('text is checked again: hidden characters removed, blank lines collapsed, capped', () => {
		expect(plainText('  A\u0000b\u001b[2J‮c​ \r\n\r\n\r\n\r\n  d \t e  \n\n')).toBe('Ab[2Jc\n\nd e');
		expect(plainText('   \n\u0000 ')).toBeNull();
		expect(plainText(42)).toBeNull();
		expect(plainText('x'.repeat(MAX_TEXT * 3))!.length).toBe(MAX_TEXT);
		// Emoji are never split in half by the cap.
		const capped = plainText('🎵'.repeat(10), 3)!;
		expect([...capped]).toEqual(['🎵', '🎵', '🎵']);
		expect(sanitiseArtistInfo({ photo: PHOTO, credits: 'Line\nbreak' }).credits).toBe('Line break');
	});

	test('hostile input never throws and never leaves hidden characters', () => {
		let x = 0x2545f491;
		const next = () => {
			x ^= x << 13;
			x ^= x >>> 17;
			x ^= x << 5;
			return x >>> 0;
		};
		const atoms = ['\n', '\r', ' ', '\t', '\u0000', '‮', 'é', '🎵', '/v1/art/', 'imageproxy/', '%2F', 'https', '/image', '..', '<b>', 'x'];
		for (let i = 0; i < 3000; i++) {
			const s = Array.from({ length: next() % 16 }, () => atoms[next() % atoms.length]).join('');
			const a = sanitiseArtistInfo({ biography: s, photo: s, credits: s });
			if (a.biography) {
				expect(a.biography).not.toMatch(/[\u0000-\u0009\u000b-\u001f‮]/);
				expect(a.biography).not.toContain('\n\n\n');
			}
			if (a.photo) expect(a.photo.startsWith('imageproxy/')).toBe(true);
			expect(() => paragraphs(s)).not.toThrow();
		}
	});

	test('ids, paragraphs and length', () => {
		expect(infoPath('artist', 7)).toBe('/v1/info/artist/7');
		expect(infoPath('album', 1)).toBe('/v1/info/album/1');
		for (const id of [0, -1, 1.5, NaN, '7', 1e13, null]) expect(infoPath('artist', id)).toBeNull();
		expect(paragraphs('a\n\nb\nc\n\n\n')).toEqual(['a', 'b\nc']);
		expect(isLong('Short.')).toBe(false);
		expect(isLong('One.\n\nTwo.')).toBe(true);
		expect(isLong('x'.repeat(300))).toBe(true);
		expect(isLong('a\nb\nc\nd')).toBe(true);
	});
});

// --- API --------------------------------------------------------------------------------------

describe('Api.info', () => {
	const session = { authed: <T>(fn: (t: string) => Promise<T>) => fn('tok') } as unknown as SessionManager;

	test('asks the gateway with the bearer token', async () => {
		const f = vi.fn(async () => new Response(JSON.stringify({ review: 'x' }), { status: 200, headers: { 'content-type': 'application/json' } }));
		vi.stubGlobal('fetch', f);
		expect(await new Api(session).info('album', 12)).toEqual({ review: 'x' });
		const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/v1/info/album/12');
		expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok');
	});

	test('bad ids are refused without asking', async () => {
		const f = vi.fn();
		vi.stubGlobal('fetch', f);
		for (const id of [0, -1, 1.5, NaN, 1e13]) await expect(new Api(session).info('artist', id)).rejects.toBeInstanceOf(ApiError);
		expect(f).not.toHaveBeenCalled();
	});
});

// --- Read more ----------------------------------------------------------------------------------

describe('InfoText', () => {
	test('long text: the first lines, then a disclosure button', async () => {
		const { container } = render(InfoText, { heading: 'Biography', text: LONG });
		const region = screen.getByRole('region', { name: 'Biography' });
		expect(region.querySelector('h2')?.textContent).toBe('Biography');
		expect(container.querySelectorAll('.text p').length).toBe(1);
		const btn = screen.getByRole('button', { name: 'Read more of the biography' });
		expect(btn.getAttribute('aria-expanded')).toBe('false');
		const controlled = document.getElementById(btn.getAttribute('aria-controls')!);
		expect(controlled?.classList.contains('text')).toBe(true);
		await fireEvent.click(btn);
		expect(btn.getAttribute('aria-expanded')).toBe('true');
		expect(btn.textContent).toBe('Show less');
		expect(container.querySelectorAll('.text p').length).toBe(2);
		expect(container.textContent).toContain('With a second line.');
		await fireEvent.click(btn);
		expect(container.querySelectorAll('.text p').length).toBe(1);
	});

	test('short text has no button; markup is shown as text', () => {
		const { container } = render(InfoText, { heading: 'Review', text: HOSTILE });
		expect(screen.queryByRole('button')).toBeNull();
		expect(container.textContent).toContain(HOSTILE);
		expect(container.querySelectorAll('img, script').length).toBe(0);
	});
});

// --- pages ------------------------------------------------------------------------------------

describe('artist page', () => {
	test('shows the photo and the biography', async () => {
		params.id = '3';
		fake.handler.rpc = async (_p, c) => (c[0] === 'albums' ? { albums_loop: [{ id: 1, album: 'Invented Album', artist: 'The Invented Band' }] } : {});
		fake.handler.info = async () => ({ biography: `${HOSTILE}\n\n${LONG}`, photo: PHOTO, credits: 'Discogs' });
		const arts: string[] = [];
		fake.api.art = (p: string) => {
			arts.push(p);
			return Promise.resolve('blob:photo');
		};
		const { container } = render(ArtistPage);
		await waitFor(() => expect(screen.getByRole('region', { name: 'Biography' })).toBeTruthy());
		expect(fake.infoCalls).toEqual([{ kind: 'artist', id: 3 }]);
		expect(container.textContent).toContain(HOSTILE);
		expect(container.querySelectorAll('script').length).toBe(0);
		expect(screen.getByRole('button', { name: 'Read more of the biography' })).toBeTruthy();
		await waitFor(() => expect(container.querySelector('[data-artist-photo] img')).not.toBeNull());
		const img = container.querySelector('[data-artist-photo] img') as HTMLImageElement;
		expect(img.getAttribute('src')).toBe('blob:photo');
		expect(img.alt).toBe('Photo of The Invented Band');
		expect(arts).toEqual(['imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image_400x400']);
		expect(container.querySelector('figcaption')?.textContent).toBe('Photo: Discogs');
		// The only img on the page is the photo, from a blob: URL.
		expect([...container.querySelectorAll('img')].every((i) => i.getAttribute('src')?.startsWith('blob:'))).toBe(true);
	});

	test('failures and missing information are simply not shown', async () => {
		params.id = '3';
		fake.handler.info = async () => {
			throw new ApiError(503, "the server didn't answer");
		};
		const { container } = render(ArtistPage);
		await settle();
		expect(fake.infoCalls.length).toBe(1);
		expect(screen.queryByRole('region', { name: 'Biography' })).toBeNull();
		expect(container.querySelector('[data-artist-photo]')).toBeNull();
		expect(container.querySelector('.error')).toBeNull();
	});

	test('a photo that fails to load is hidden', async () => {
		params.id = '3';
		fake.handler.info = async () => ({ biography: null, photo: PHOTO, credits: 'Discogs' });
		fake.api.art = () => Promise.reject(new ApiError(404, 'art HTTP 404'));
		const { container } = render(ArtistPage);
		await settle();
		expect(container.querySelector('[data-artist-photo]')).toBeNull();
		expect(container.textContent).not.toContain('Discogs');
	});

	test('not asked on a role page or for a bad id', async () => {
		params.id = '3';
		params.role = 'composer';
		render(ArtistPage);
		await settle();
		params.role = undefined;
		params.id = '../1';
		render(ArtistPage);
		await settle();
		expect(fake.infoCalls).toEqual([]);
	});
});

describe('album page', () => {
	const album = async () => {
		params.id = '8';
		fake.handler.rpc = async (_p, c) =>
			c[0] === 'albums' ? { albums_loop: [{ id: 8, album: 'Invented Album', artist: 'The Invented Band' }] } : { titles_loop: [{ id: 81, title: 'One' }] };
		const r = render(AlbumPage);
		await screen.findByRole('heading', { name: 'Invented Album' });
		await settle();
		return r;
	};

	test('shows the review, collapsed', async () => {
		fake.handler.info = async () => ({ review: LONG });
		await album();
		expect(fake.infoCalls).toEqual([{ kind: 'album', id: 8 }]);
		const region = screen.getByRole('region', { name: 'Review' });
		expect(region.textContent).toContain('First paragraph');
		expect(region.textContent).not.toContain('Second paragraph');
		await fireEvent.click(screen.getByRole('button', { name: 'Read more of the review' }));
		expect(region.textContent).toContain('Second paragraph');
	});

	test('no review, or a failure: no section', async () => {
		fake.handler.info = async () => ({ review: null });
		await album();
		expect(screen.queryByRole('region', { name: 'Review' })).toBeNull();
		fake.handler.info = async () => {
			throw new ApiError(429, 'busy');
		};
		const { container } = await album();
		expect(screen.queryByRole('region', { name: 'Review' })).toBeNull();
		expect(container.querySelector('.error')).toBeNull();
	});
});
