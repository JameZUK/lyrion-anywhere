// "Your year in sound": the story's cards, and sharing one as an image made on the device
// (a fake canvas stands in for the browser's; a fake navigator.share for the share sheet).
import { emptyStats, type ListeningStats } from '$lib/api/me';
import { noNames } from '$lib/stats/names';
import { storyCards, type StoryCard } from '$lib/stats/story';
import { IMAGE, cardImage, drawCard, imageName, shareCard, wrap } from '$lib/stats/share';
import { fakeCanvas } from './helpers/fake-canvas';

const CARD: StoryCard = { key: 'artist', kicker: 'Your top artist', headline: 'An Example Band', detail: '40 plays, 2.5 hours.' };

function yearStats(): ListeningStats {
	const s = emptyStats(true, 396);
	s.totals = { plays: 412, seconds: 82800, tracks: 301, albums: 64, artists: 41, days: 27 };
	s.top_artists = [
		{ id: 1, name: 'An Example Band', plays: 40, seconds: 9100 },
		{ id: 2, name: '', plays: 30, seconds: 8000 },
		{ id: 3, name: 'Third Example', plays: 20, seconds: 4000 },
	];
	s.top_albums = [{ id: 7, plays: 22, seconds: 5200 }];
	s.top_tracks = [{ id: 9, plays: 6, seconds: 1300 }];
	s.new_artists = [{ id: 31, name: 'Another Example', first_played: null }];
	s.by_hour[21] = 40;
	s.longest_streak = { days: 9, from: '2026-10-02', to: '2026-10-10' };
	return s;
}

test('the story’s cards, in order; ones with nothing to say are left out', () => {
	const names = noNames();
	names.albums.set(7, { id: 7, album: 'Blue Example', artist: 'An Example Band' });
	const cards = storyCards(2026, yearStats(), names);
	expect(cards.map((c) => c.key)).toEqual(['hours', 'artist', 'album', 'track', 'five', 'discovery', 'hour', 'streak', 'summary']);
	expect(cards[0]!.headline).toBe('23 hours');
	expect(cards[2]!.headline).toBe('Blue Example');
	expect(cards[3]!.headline).toBe('A track no longer in the library');
	expect(cards[4]!.list).toEqual(['An Example Band', 'An unnamed artist', 'Third Example']);
	expect(cards[6]!.headline).toBe('21:00–22:00');
	expect(cards[7]!.detail).toBe('From 2 October 2026 to 10 October 2026.');
	const bare = emptyStats(true);
	bare.totals.plays = 3;
	expect(storyCards(2026, bare, noNames()).map((c) => c.key)).toEqual(['hours', 'summary']);
});

test('wrap: lines fit the width, and long text is cut with an ellipsis', () => {
	const ctx = { measureText: (t: string) => ({ width: t.length * 10 }) };
	expect(wrap(ctx, 'one two three four', 90)).toEqual(['one two', 'three', 'four']);
	expect(wrap(ctx, 'a b c d e f', 10, 2)).toEqual(['a', 'b…']);
	expect(wrap(ctx, '', 100)).toEqual([]);
});

test('drawing: text only, on a 1080 × 1350 image, with the footer', async () => {
	const c = fakeCanvas();
	const blob = await cardImage({ ...CARD, headline: '<script>alert(1)</script>' }, 'Your year in sound 2026 · Lyrion Anywhere', () => c);
	expect([c.width, c.height]).toEqual([IMAGE.width, IMAGE.height]);
	expect(blob.type).toBe('image/png');
	expect(c.texts).toContain('Your top artist');
	expect(c.texts.join(' ')).toContain('<script>alert(1)</script>');
	expect(c.texts.at(-1)).toBe('Your year in sound 2026 · Lyrion Anywhere');
	// The background fills the whole image first.
	expect(c.rects[0]).toEqual([0, 0, IMAGE.width, IMAGE.height]);
	const list = fakeCanvas();
	drawCard(list.getContext('2d')!, { ...CARD, list: ['A', 'B', 'C', 'D', 'E', 'F'] }, 'f');
	expect(list.texts.filter((t) => /^\d\. /.test(t))).toEqual(['1. A', '2. B', '3. C', '4. D', '5. E']);
});

test('no 2D canvas: a clear error, nothing shared', async () => {
	const c = fakeCanvas();
	c.getContext = () => null;
	const share = vi.fn();
	await expect(shareCard(CARD, 2026, { makeCanvas: () => c, nav: { share, canShare: () => true } })).rejects.toThrow('can’t make images');
	expect(share).not.toHaveBeenCalled();
});

test('share sheet with a file where the browser can share files', async () => {
	const share = vi.fn(async (_d: ShareData) => {});
	const canShare = vi.fn((d: ShareData) => !!d.files?.length);
	const r = await shareCard(CARD, 2026, { makeCanvas: fakeCanvas, nav: { share, canShare } });
	expect(r).toBe('shared');
	const sent = share.mock.calls[0]![0];
	expect(sent.files).toHaveLength(1);
	expect(sent.files![0]!.name).toBe('your-year-2026-artist.png');
	expect(sent.files![0]!.type).toBe('image/png');
	expect(sent.title).toBe('Your year in sound 2026');
	// Only the image: no link, nothing that points back at the server.
	expect(sent.url).toBeUndefined();
	expect(sent.text).toBeUndefined();
});

test('the user closing the share sheet is not an error, and nothing is saved', async () => {
	const createObjectURL = vi.fn(() => 'blob:x');
	const share = vi.fn(async () => {
		throw new DOMException('closed', 'AbortError');
	});
	const r = await shareCard(CARD, 2026, { makeCanvas: fakeCanvas, nav: { share, canShare: () => true }, url: { createObjectURL, revokeObjectURL: vi.fn() } });
	expect(r).toBe('cancelled');
	expect(createObjectURL).not.toHaveBeenCalled();
});

test('no file sharing: the PNG is saved as a download instead', async () => {
	vi.useFakeTimers();
	try {
		const createObjectURL = vi.fn((_b: Blob) => 'blob:local-1');
		const revokeObjectURL = vi.fn();
		const clicked: { href: string; download: string }[] = [];
		const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
			clicked.push({ href: this.getAttribute('href') ?? '', download: this.download });
		});
		for (const nav of [null, { share: vi.fn(), canShare: () => false }, { share: vi.fn(async () => Promise.reject(new DOMException('no', 'NotAllowedError'))), canShare: () => true }]) {
			const r = await shareCard(CARD, 2026, { makeCanvas: fakeCanvas, nav, url: { createObjectURL, revokeObjectURL } });
			expect(r).toBe('saved');
		}
		expect(clicked).toEqual(Array(3).fill({ href: 'blob:local-1', download: 'your-year-2026-artist.png' }));
		expect(createObjectURL.mock.calls[0]![0].type).toBe('image/png');
		// The link is gone at once; the object URL is let go a little later.
		expect(document.querySelector('a[download]')).toBeNull();
		vi.advanceTimersByTime(10_000);
		expect(revokeObjectURL).toHaveBeenCalledWith('blob:local-1');
		click.mockRestore();
	} finally {
		vi.useRealTimers();
	}
});

test('image names are safe whatever the card key', () => {
	expect(imageName(2026, { ...CARD, key: '../../etc/passwd' })).toBe('your-year-2026--etc-passwd.png');
	expect(imageName(2026.7, { ...CARD, key: '' })).toBe('your-year-2026-card.png');
});
