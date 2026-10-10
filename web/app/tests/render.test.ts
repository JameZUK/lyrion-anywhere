// Library metadata is attacker-controlled (anyone can tag a file). Rendered with hostile
// names and ids, components must show them as text and never create markup or bad links.
import { render } from '@testing-library/svelte';
import TrackList from '$lib/components/TrackList.svelte';
import AlbumGrid from '$lib/components/AlbumGrid.svelte';
import type { Album, Track } from '$lib/lms/types';

const HOSTILE = [
	'<img src=x onerror=alert(1)>',
	'<script>alert(1)</script>',
	'"><svg onload=alert(1)>',
	"javascript:alert('x')",
	'{@html "<b>x</b>"}',
	'‮evil‬',
];

beforeAll(() => {
	// jsdom has no IntersectionObserver; art then simply stays a placeholder.
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

function assertInert(container: HTMLElement) {
	expect(container.querySelectorAll('script, img, svg[onload], iframe, object, embed').length).toBe(0);
	for (const el of container.querySelectorAll('*')) {
		for (const attr of el.attributes) {
			expect(attr.name.startsWith('on'), `${el.tagName} has ${attr.name}`).toBe(false);
			if (attr.name === 'href') expect(attr.value).toMatch(/^#\//);
		}
	}
}

test('track list shows hostile titles and artists as text', () => {
	const tracks: Track[] = HOSTILE.map((h, i) => ({ id: i + 1, title: h, artist: h, duration: 60, tracknum: h }));
	const { container } = render(TrackList, { tracks, onplay: () => {}, onnext: () => {}, onadd: () => {}, showArtist: true });
	assertInert(container);
	for (const h of HOSTILE) expect(container.textContent).toContain(h.slice(0, 20));
});

test('album grid shows hostile names as text and keeps links in the app', () => {
	const albums = HOSTILE.map((h, i) => ({
		id: i % 2 ? i + 1 : (h as unknown as number), // hostile ids too
		album: h,
		artist: h,
		artwork_track_id: h,
		year: h,
	})) as Album[];
	const { container } = render(AlbumGrid, { albums });
	assertInert(container);
	const hrefs = [...container.querySelectorAll('a')].map((a) => a.getAttribute('href'));
	for (const h of hrefs) expect(h).toMatch(/^#\/(album\/\d+|albums)$/);
	for (const h of HOSTILE) expect(container.textContent).toContain(h.slice(0, 20));
});

test('over-long names are cut, not rendered in full', () => {
	const { container } = render(TrackList, { tracks: [{ id: 1, title: 'x'.repeat(10_000) }], onplay: () => {} });
	expect(container.textContent!.length).toBeLessThan(1000);
});
