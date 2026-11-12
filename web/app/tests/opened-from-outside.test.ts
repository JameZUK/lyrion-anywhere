// Review (Web low): `#/go/radio`, `#/go/resume` and `#/search-on/…` acted as soon as they
// loaded, so any link elsewhere could start radio, resume playback or run a search on the
// user's player. They now ask for a tap when the route was opened from outside the app (a link,
// the address bar, a bookmark, the installed app's shortcuts, a reload), and act at once only
// when reached by a tap in the app. This is the record of which is which.
import { goto } from '$app/navigation';
import { noteNavigation, openedFromOutside, routeOutsideHashChanges } from '#lib/nav';

/** The history state of an entry SvelteKit 3's router made. */
const ROUTER = { 'sveltekit:metadata': { historyIndex: 5, navigationIndex: 5 } };

afterEach(() => history.replaceState(null, '', '#/'));

test('the page this load opened on is from outside; a tap in the app is not', () => {
	history.replaceState(null, '', '#/go/radio');
	noteNavigation({ type: 'enter' });
	expect(openedFromOutside('#/go/radio')).toBe(true);
	expect(openedFromOutside('#/go/resume')).toBe(false);
	// Followed a link in the app, or the app went there itself.
	for (const type of ['link', 'goto']) {
		noteNavigation({ type: 'enter' });
		history.replaceState(null, '', '#/search-on/examplemusic/blue%20moon');
		noteNavigation({ type });
		expect(openedFromOutside('#/search-on/examplemusic/blue%20moon'), type).toBe(false);
	}
	// Back and Forward through the app's own pages count as the app's.
	noteNavigation({ type: 'popstate', delta: -1 });
	expect(openedFromOutside(location.hash)).toBe(false);
});

test('read late (after the next navigation), it still answers for the route asked about', () => {
	history.replaceState(null, '', '#/search-on/examplemusic/a');
	noteNavigation({ type: 'enter' });
	// The app went to another search; the page asks before the record has caught up.
	expect(openedFromOutside('#/search-on/examplemusic/b')).toBe(false);
	expect(openedFromOutside('#/search-on/examplemusic/a')).toBe(true);
	// An anchor on the page doesn't make it another route.
	expect(openedFromOutside('#/search-on/examplemusic/a#top')).toBe(true);
});

test('a route opened by changing the address outside the router counts as from outside', async () => {
	const back = vi.spyOn(history, 'back').mockImplementation(() => {});
	vi.mocked(goto).mockClear();
	history.replaceState(ROUTER, '', '#/');
	noteNavigation({ type: 'enter' });
	noteNavigation({ type: 'link' }); // somewhere in the app first
	const stop = routeOutsideHashChanges(() => '#/');
	try {
		history.replaceState(null, '', '#/go/resume');
		dispatchEvent(new PopStateEvent('popstate', { state: null }));
		dispatchEvent(new PopStateEvent('popstate', { state: ROUTER }));
		await new Promise((r) => setTimeout(r, 0));
		expect(goto).toHaveBeenCalledWith('#/go/resume');
		// The router's afterNavigate for that goto.
		noteNavigation({ type: 'goto' });
		expect(openedFromOutside('#/go/resume')).toBe(true);
		// The next navigation in the app is the app's again.
		history.replaceState(null, '', '#/queue');
		noteNavigation({ type: 'link' });
		expect(openedFromOutside('#/queue')).toBe(false);
		expect(openedFromOutside('#/go/resume')).toBe(false);
	} finally {
		stop();
		back.mockRestore();
	}
});

test('junk never counts as a route opened from outside unless it is that route', () => {
	history.replaceState(null, '', '#/go/radio');
	noteNavigation({ type: 'enter' });
	for (const junk of ['', '#', '#/', '/go/radio/', '#/go/RADIO', '#//go/radio', '#/go/radio?x']) {
		expect(openedFromOutside(junk), junk).toBe(false);
	}
	expect(openedFromOutside('/go/radio')).toBe(true); // with or without the '#'
});
