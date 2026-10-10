// LMS menus: resolving actions the way the gateway does, and refusing malformed ones.
import { itemKind, needsInput, opensMenu, resolveAction, toCommand, type SBMenu } from '$lib/lms/slimbrowse';

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
		{ text: 'Similar', type: 'redirect', actions: { go: { cmd: ['audiomuse', 'similar_track'], params: { track_id: 9 } } } },
		{ text: 'A review of the album, long text…', type: 'text' },
	],
};
const items = menu.item_loop!;

test('own actions win; base actions combine with the item’s parameters', () => {
	expect(resolveAction(menu, items[0]!, 'go')).toEqual({ cmd: ['radiofeeds', 'items'], params: { item_id: 'a.0', menu: 'radiofeeds' } });
	expect(resolveAction(menu, items[1]!, 'play')).toEqual({
		cmd: ['radiofeeds', 'playlist', 'play'],
		params: { menu: 'radiofeeds', item_id: 'a.1.4' },
		nextWindow: 'nowPlaying',
	});
	// "more" takes the item's infoParams.
	expect(resolveAction(menu, items[1]!, 'more')).toEqual({ cmd: ['podcastinfo', 'items'], params: { menu: '1', item_id: 'a.1.4' } });
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
