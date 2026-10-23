// Each route has its own document title (announced by SvelteKit after navigation), and detail
// pages have a Back control (an installed iPhone web app has no browser Back).
import { fireEvent, render } from '@testing-library/svelte';
import { goto } from '$app/navigation';
import { APP_NAME, documentTitle, sectionTitle } from '#lib/title.svelte';
import { back, canGoBack, noteNavigation } from '#lib/nav';
import BackButton from '#lib/components/BackButton.svelte';

describe('titles', () => {
	test('every section has its own title', () => {
		expect(documentTitle('/')).toBe('Now playing – Lyrion Anywhere');
		expect(documentTitle('/search')).toBe('Search – Lyrion Anywhere');
		expect(documentTitle('/settings')).toBe('Settings – Lyrion Anywhere');
		expect(documentTitle('/album/12')).toBe('Album – Lyrion Anywhere');
		expect(documentTitle('/favourites/abc.1')).toBe('Favourites – Lyrion Anywhere');
		expect(sectionTitle('#/new')).toBe('New music');
		expect(sectionTitle('/search?q=x')).toBe('Search');
		expect(documentTitle('/no-such-page')).toBe(APP_NAME);
	});

	test('detail pages name their item, tidily and briefly', () => {
		expect(documentTitle('/album/12', 'Abbey Road')).toBe('Abbey Road – Lyrion Anywhere');
		expect(documentTitle('/album/12', 'A\n\tB')).toBe('A B – Lyrion Anywhere');
		expect(documentTitle('/album/12', 'x'.repeat(500)).length).toBeLessThan(150);
		// Shown as text by the browser: nothing is interpreted.
		expect(documentTitle('/album/1', '<img src=x onerror=alert(1)>')).toBe('<img src=x onerror=alert(1)> – Lyrion Anywhere');
	});
});

describe('back', () => {
	beforeEach(() => {
		noteNavigation({ type: 'enter' });
		vi.mocked(goto).mockClear();
	});

	test('opened directly: Back goes up to the parent, replacing this page', async () => {
		expect(canGoBack()).toBe(false);
		await back('/albums');
		expect(goto).toHaveBeenCalledWith('#/albums', { replaceState: true });
		expect(canGoBack()).toBe(false);
	});

	test('after moving around the app, Back goes back through its history', async () => {
		const spy = vi.spyOn(history, 'back').mockImplementation(() => {});
		noteNavigation({ type: 'link' });
		noteNavigation({ type: 'goto' });
		expect(canGoBack()).toBe(true);
		await back('/albums');
		expect(spy).toHaveBeenCalledTimes(1);
		expect(goto).not.toHaveBeenCalled();
		noteNavigation({ type: 'popstate', delta: -1 });
		noteNavigation({ type: 'popstate', delta: -1 });
		expect(canGoBack()).toBe(false);
		noteNavigation({ type: 'popstate', delta: -5 }); // never below zero
		expect(canGoBack()).toBe(false);
		spy.mockRestore();
	});

	test('the Back control links to the parent, and navigates in-app', async () => {
		const r = render(BackButton, { parent: '/albums' });
		const a = r.getByRole('link', { name: 'Back' });
		expect(a.getAttribute('href')).toBe('#/albums');
		await fireEvent.click(a);
		expect(goto).toHaveBeenCalledWith('#/albums', { replaceState: true });
	});
});
