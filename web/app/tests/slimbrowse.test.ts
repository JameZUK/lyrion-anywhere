// LMS menus: resolving actions the way the gateway does, and refusing malformed ones.
import { itemKind, needsInput, opensMenu, pluginEntries, resolveAction, toCommand, type SBMenu } from '$lib/lms/slimbrowse';

const menu: SBMenu = {
	base: {
		actions: {
			go: { cmd: ['radiofeeds', 'items'], params: { menu: 'radiofeeds' }, itemsParams: 'params' },
			play: { cmd: ['radiofeeds', 'playlist', 'play'], params: { menu: 'radiofeeds' }, itemsParams: 'params', nextWindow: 'nowPlaying' },
			add: { cmd: ['radiofeeds', 'playlist', 'add'], params: { menu: 'radiofeeds' }, itemsParams: 'params' },
			more: { cmd: ['podcastinfo', 'items'], params: { menu: 1 }, itemsParams: 'infoParams' },
		},
	},
	item_loop: [
		{ text: 'By name', type: 'link', actions: { go: { cmd: ['radiofeeds', 'items'], params: { item_id: 'a.0', menu: 'radiofeeds' } } } },
		{ text: 'Station', type: 'audio', params: { item_id: 'a.1.4' }, infoParams: { item_id: 'a.1.4' } },
		{ text: 'Search', type: 'search', actions: { go: { cmd: ['podcasts', 'items'], params: { search: '__TAGGEDINPUT__', item_id: '1' } } } },
		{ text: 'Similar', type: 'redirect', nextWindow: 'nowPlaying', actions: { go: { cmd: ['audiomuse', 'similar_track'], params: { track_id: 9 } } } },
		{ text: 'A review of the album, long text…', type: 'text' },
	],
};
const items = menu.item_loop!;

test('own actions win; base actions combine with the item’s parameters', () => {
	expect(resolveAction(menu, items[0]!, 'go')).toEqual({ cmd: ['radiofeeds', 'items'], params: { item_id: 'a.0', menu: 'radiofeeds' }, menu: true });
	expect(resolveAction(menu, items[1]!, 'play')).toEqual({
		cmd: ['radiofeeds', 'playlist', 'play'],
		params: { menu: 'radiofeeds', item_id: 'a.1.4' },
		nextWindow: 'nowPlaying',
		menu: false,
	});
	// "more" takes the item's infoParams.
	expect(resolveAction(menu, items[1]!, 'more')).toEqual({ cmd: ['podcastinfo', 'items'], params: { menu: '1', item_id: 'a.1.4' }, menu: true });
	// No params field for a base action: not offered.
	expect(resolveAction(menu, items[0]!, 'more')).toBeNull();
	expect(resolveAction(menu, items[4]!, 'go')).toBeNull();
});

test('commands: paging for menus only, search text filled in and cleaned', () => {
	const go = resolveAction(menu, items[0]!, 'go')!;
	expect(opensMenu(go)).toBe(true);
	expect(toCommand(go, { start: 100, count: 50 })).toEqual(['radiofeeds', 'items', 100, 50, 'item_id:a.0', 'menu:radiofeeds']);
	const similar = resolveAction(menu, items[3]!, 'go')!;
	expect(toCommand(similar)).toEqual(['audiomuse', 'similar_track', 'track_id:9']);
	const search = resolveAction(menu, items[2]!, 'go')!;
	expect(needsInput(search)).toBe(true);
	expect(toCommand(search, undefined, ' in our\ntime ')).toContain('search:in our time');
	expect(String(toCommand(search, undefined, 'x'.repeat(500)).find((x) => String(x).startsWith('search:'))).length).toBeLessThanOrEqual(207);
	expect(toCommand(go, { start: -5, count: 9999 }).slice(2, 4)).toEqual([0, 500]);
});

test('items are classified for display', () => {
	expect(items.map((i) => itemKind(menu, i))).toEqual(['folder', 'playable', 'search', 'action', 'text']);
});

test('malformed or dangerous actions are not offered at all', () => {
	const bad: SBMenu = {
		item_loop: [
			{ actions: { go: { cmd: 'radiofeeds items' } } },
			{ actions: { go: { cmd: [] } } },
			{ actions: { go: { cmd: ['radio feeds', 'items'] } } },
			{ actions: { go: { cmd: ['a', 'b', 'c', 'd', 'e', 'f', 'g'] } } },
			{ actions: { go: { cmd: ['x', 'items'], params: { url: 'file:///etc/passwd' } } } },
			{ actions: { go: { cmd: ['x', 'items'], params: { 'bad key': 1 } } } },
			{ actions: { go: { cmd: ['x;rm', 'items'] } } },
			{ actions: { go: null } },
		],
	};
	for (const i of bad.item_loop!) expect(resolveAction(bad, i, 'go')).toBeNull();
	// Non-scalar values are skipped, not stringified into commands.
	const odd: SBMenu = { item_loop: [{ actions: { go: { cmd: ['x', 'items'], params: { a: { b: 1 }, c: [1], d: 'ok' } } } }] };
	expect(resolveAction(odd, odd.item_loop![0]!, 'go')!.params).toEqual({ d: 'ok' });
});

// A plugin menu in LMS's Jive style (as AudioMuse-AI's): `go` opens a submenu unless the item
// says `nextWindow`, in which case it runs.
const plugin: SBMenu = {
	item_loop: [
		{ text: 'Browse by mood', actions: { go: { cmd: ['someplugin', 'menu_mood'], player: 0 } } },
		{ text: 'Similar to current track', nextWindow: 'refresh', actions: { go: { cmd: ['someplugin', 'similar_now'], player: 0 } } },
		{ text: 'Describe it', nextWindow: 'refresh', input: { len: 1 }, actions: { go: { cmd: ['someplugin', 'instant'], params: { prompt: '__TAGGEDINPUT__' } } } },
	],
};

test('a go without nextWindow opens a menu, paged; with nextWindow it runs as a command', () => {
	const [mood, similar, describe] = plugin.item_loop!.map((i) => resolveAction(plugin, i, 'go')!);
	expect(opensMenu(mood!)).toBe(true);
	expect(toCommand(mood!, { start: 0, count: 100 })).toEqual(['someplugin', 'menu_mood', 0, 100]);
	expect(itemKind(plugin, plugin.item_loop![0]!)).toBe('folder');
	expect(opensMenu(similar!)).toBe(false);
	expect(similar!.nextWindow).toBe('refresh');
	expect(toCommand(similar!, { start: 0, count: 100 })).toEqual(['someplugin', 'similar_now']);
	expect(itemKind(plugin, plugin.item_loop![1]!)).toBe('action');
	expect(itemKind(plugin, plugin.item_loop![2]!)).toBe('search');
	expect(toCommand(describe!, undefined, 'quiet piano')).toEqual(['someplugin', 'instant', 'prompt:quiet piano']);
});

test('plugin entries come from the home menu, minus what the app shows elsewhere', () => {
	const home: SBMenu = {
		item_loop: [
			{ id: 'pluginSomePlugin', node: 'myMusic', text: 'Some Plugin', weight: 80, icon: 'plugins/Some/icon.png', actions: { go: { cmd: ['someplugin', 'menu'], player: 0 } } },
			{ id: 'randomplay', node: 'myMusic', text: 'Random Mix', weight: 60, isANode: 1 },
			{ id: 'randomtracks', node: 'randomplay', text: 'Song Mix', nextWindow: 'nowPlaying', actions: { go: { cmd: ['randomplay', 'tracks'] } } },
			{ id: 'myMusicAlbums', node: 'myMusic', text: 'Albums', actions: { go: { cmd: ['browselibrary', 'items'], params: { mode: 'albums' } } } },
			{ id: 'opmlspotty', node: '', text: 'Spotty', actions: { go: { cmd: ['spotty', 'items'], params: { menu: 'spotty' } } } },
			{ id: 'opml', node: 'plugins', text: 'Broken', actions: { go: { cmd: [null, 'items'] } } },
			{ id: 'emptynode', node: 'myMusic', text: 'Nothing here' },
			{ id: 'stopper', node: 'myMusic', text: 'Run a thing', nextWindow: 'refresh', actions: { go: { cmd: ['x', 'y'] } } },
			{ id: '<bad id>', node: 'myMusic', text: 'Bad', actions: { go: { cmd: ['z', 'menu'] } } },
		],
	};
	const got = pluginEntries(home, new Set(['spotty']));
	expect(got.map((e) => e.id)).toEqual(['randomplay', 'pluginSomePlugin']);
	expect(got[0]!.go).toBeNull();
	expect(got[0]!.children.map((c) => c.id)).toEqual(['randomtracks']);
	expect(got[1]!.go?.cmd).toEqual(['someplugin', 'menu']);
	expect(got[1]!.icon).toBe('plugins/Some/icon.png');
	expect(pluginEntries({}, new Set())).toEqual([]);
	expect(pluginEntries({ item_loop: [null as never, 5 as never, { id: 7 } as never] }, new Set())).toEqual([]);
});
