// "Start radio" on the web (car mode and the Radio shortcut): AudioMuse-AI's mix picked out of
// a track's context menu and run exactly as offered, Don't Stop The Music read from its own
// menu (untrusted) and switched on only when off; no radio when the server offers neither.
import { render, waitFor } from '@testing-library/svelte';
import { goto } from '$app/navigation';
import { audiomuseOffers, chooseProvider, dstmFrom, isAudioMuse, providerOk, radioFromTrack, radioPlan, startRadio, DSTM_PREF } from '#lib/lms/discovery';
import type { SBMenu } from '#lib/lms/slimbrowse';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string> = { action: 'resume' };
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/go/resume') } }));
const fake = await import('./helpers/fake-app.svelte');
const { default: GoPage } = await import('../src/routes/go/[action]/+page.svelte');

const PLAYER = '02:00:00:00:00:01';
const ctx = (items: unknown[]): SBMenu => ({ count: items.length, item_loop: items as SBMenu['item_loop'] });

const MIX_MENU = ctx([
	{ text: 'Play next', actions: { go: { cmd: ['playlistcontrol'], params: { cmd: 'insert', track_id: 9 } } }, nextWindow: 'parent' },
	{ text: 'Similar tracks', actions: { go: { cmd: ['audiomuseai', 'similar'], params: { track_id: 9 } } } },
	{ text: 'Instant mix', nextWindow: 'nowPlaying', actions: { go: { cmd: ['audiomuseai', 'mix'], params: { track_id: 9 } } } },
	{ text: 'Some other plugin mix', nextWindow: 'nowPlaying', actions: { go: { cmd: ['otherplugin', 'mix'], params: { track_id: 9 } } } },
]);
const DSTM_MENU = {
	item_loop: [
		{ text: 'Off', radio: 0, actions: { do: { cmd: ['playerpref', DSTM_PREF, '0'] } } },
		{ text: 'Random tracks', radio: 0, actions: { do: { cmd: ['playerpref', DSTM_PREF, 'PLUGIN_RANDOM_TRACK'] } } },
		{ text: 'Similar music', radio: 0, actions: { do: { cmd: ['playerpref', DSTM_PREF, 'PLUGIN_DSTM_SIMILAR'] } } },
	],
};

describe('picking out what the server offers', () => {
	test('AudioMuse-AI’s mix, not other plugins’ or core entries', () => {
		const o = audiomuseOffers(MIX_MENU);
		expect(o.mix?.label).toBe('Instant mix');
		expect(o.mix?.action.cmd).toEqual(['audiomuseai', 'mix']);
		expect(o.mix?.action.menu).toBe(false);
		const none = audiomuseOffers(
			ctx([
				{ text: 'Mix', nextWindow: 'nowPlaying', actions: { go: { cmd: ['randomplay', 'tracks'] } } },
				{ text: 'Search AudioMuse', actions: { go: { cmd: ['audiomuseai', 'search'], params: { search: '__TAGGEDINPUT__' } } } },
			]),
		);
		expect(none).toEqual({ mix: null, folder: null });
	});

	test('a playable similar list makes a mix; an unknown AudioMuse-AI menu is a folder to look in', () => {
		const o = audiomuseOffers(
			ctx([
				{
					text: 'Sonically similar',
					type: 'playlist',
					actions: { go: { cmd: ['audiomuseai', 'list'], params: { album_id: 4 } }, play: { cmd: ['audiomuseai', 'playlist'], params: { album_id: 4, cmd: 'play' } } },
				},
			]),
		);
		expect(o.mix?.action.cmd).toEqual(['audiomuseai', 'playlist']);
		const f = audiomuseOffers(ctx([{ text: 'AudioMuse-AI', actions: { go: { cmd: ['audiomuseai', 'menu'] } } }]));
		expect(f.mix).toBeNull();
		expect(f.folder?.cmd).toEqual(['audiomuseai', 'menu']);
	});

	test('Don’t Stop The Music: providers and the one in use; anything else is ignored', () => {
		expect(dstmFrom(DSTM_MENU)).toEqual({
			providers: [
				{ token: 'PLUGIN_RANDOM_TRACK', name: 'Random tracks' },
				{ token: 'PLUGIN_DSTM_SIMILAR', name: 'Similar music' },
			],
			current: null,
		});
		const odd = dstmFrom({
			item_loop: [
				{ text: 'Other pref', actions: { do: { cmd: ['playerpref', 'plugin.other:x', 'PLUGIN_X'] } } },
				{ text: 'Bad token', actions: { do: { cmd: ['playerpref', DSTM_PREF, 'rm -rf /'] } } },
				{ text: 'Too long', actions: { do: { cmd: ['playerpref', DSTM_PREF, 'P'.repeat(65)] } } },
				{ text: 'Extra words', actions: { do: { cmd: ['playerpref', DSTM_PREF, 'PLUGIN_A', 'more'] } } },
				{ text: 'In use', radio: '1', actions: { go: { cmd: ['playerpref', DSTM_PREF, 'PLUGIN_B'] } } },
				'junk',
			],
		});
		expect(odd).toEqual({ providers: [{ token: 'PLUGIN_B', name: 'In use' }], current: 'PLUGIN_B' });
		expect(dstmFrom({})).toBeNull();
		expect(dstmFrom(null)).toBeNull();
		expect(dstmFrom({ item_loop: [] })).toBeNull();
	});

	test('the provider: the one in use stands; otherwise similar before random', () => {
		const s = dstmFrom(DSTM_MENU)!;
		expect(chooseProvider(s)).toEqual({ provider: { token: 'PLUGIN_DSTM_SIMILAR', name: 'Similar music' }, enable: true });
		expect(chooseProvider({ ...s, current: 'PLUGIN_RANDOM_TRACK' })).toEqual({ provider: { token: 'PLUGIN_RANDOM_TRACK', name: 'Random tracks' }, enable: false });
		expect(chooseProvider({ providers: [], current: null })).toBeNull();
	});

	test('token and word checks', () => {
		expect(providerOk('PLUGIN_DSTM_SIMILAR')).toBe(true);
		for (const bad of ['', '1ABC', 'A-B', 'A B', 'x'.repeat(65), 5]) expect(providerOk(bad)).toBe(false);
		expect(isAudioMuse('AudioMuseAI')).toBe(true);
		expect(isAudioMuse('audiomus')).toBe(false);
	});

	test('robustness: odd menus never throw', () => {
		for (const v of [null, {}, { item_loop: 'x' }, { item_loop: [null, 1, 'a', [], { actions: 5 }, { actions: { go: { cmd: 'x' } } }] }]) {
			expect(() => audiomuseOffers(v as SBMenu)).not.toThrow();
			expect(() => dstmFrom(v)).not.toThrow();
		}
	});
});

describe('starting radio', () => {
	function server(menu: unknown, dstm: unknown | Error) {
		const sent: (string | number)[][] = [];
		const rpc = async (_p: string, c: (string | number)[]) => {
			sent.push(c);
			if (String(c[0]).endsWith('info')) return menu;
			if (c[0] === 'dontstopthemusicsetting') {
				if (dstm instanceof Error) throw dstm;
				return dstm;
			}
			return {};
		};
		return { rpc, sent };
	}

	test('with a mix and Don’t Stop The Music off: switch it on, then run the mix as offered', async () => {
		const s = server(MIX_MENU, DSTM_MENU);
		expect(await radioFromTrack(s.rpc, PLAYER, 9)).toBe('');
		expect(s.sent[0]).toEqual(['trackinfo', 'items', 0, 100, 'track_id:9', 'menu:1']);
		expect(s.sent.slice(2)).toEqual([
			['playerpref', DSTM_PREF, 'PLUGIN_DSTM_SIMILAR'],
			['audiomuseai', 'mix', 'track_id:9'],
		]);
	});

	test('no mix, Don’t Stop The Music already on: just play the track', async () => {
		const on = { item_loop: [{ text: 'Similar', radio: 1, actions: { do: { cmd: ['playerpref', DSTM_PREF, 'PLUGIN_DSTM_SIMILAR'] } } }] };
		const s = server(ctx([]), on);
		expect(await radioFromTrack(s.rpc, PLAYER, 9)).toBe('');
		expect(s.sent.slice(2)).toEqual([['playlistcontrol', 'cmd:load', 'track_id:9']]);
	});

	test('neither: no radio, and nothing is played', async () => {
		const s = server(ctx([]), new Error('refused'));
		expect(await radioPlan(s.rpc, PLAYER, 9)).toBeNull();
		expect(await radioFromTrack(s.rpc, PLAYER, 9)).toMatch(/AudioMuse-AI or Don’t Stop The Music/);
		expect(s.sent.every((c) => String(c[0]).endsWith('info') || c[0] === 'dontstopthemusicsetting')).toBe(true);
	});

	test('a plan that would open a menu, or a bad provider, is refused', async () => {
		const s = server(ctx([]), null);
		await expect(startRadio(s.rpc, PLAYER, 9, { mix: { cmd: ['audiomuseai', 'items'], params: {}, menu: true }, mixLabel: null, dstm: null, enableDstm: false })).rejects.toThrow();
		await expect(startRadio(s.rpc, PLAYER, 9, { mix: null, mixLabel: null, dstm: { token: 'bad token', name: 'x' }, enableDstm: true })).rejects.toThrow();
		expect(s.sent).toEqual([]);
		await expect(radioPlan(s.rpc, PLAYER, 0)).rejects.toThrow();
	});
});

describe('the Resume and Radio shortcuts', () => {
	beforeEach(() => {
		fake.calls.length = 0;
		fake.app.players = [{ playerid: PLAYER, name: 'Test player' }];
		fake.app.playerId = PLAYER;
		fake.app.notice = '';
		fake.app.now = { mode: 'pause', time: 20, duration: 200, playlist_loop: [{ id: 9, title: 'A song' }] };
		vi.mocked(goto).mockClear();
	});

	test('Resume plays once the player is there, then shows Now playing', async () => {
		params.action = 'resume';
		render(GoPage);
		await waitFor(() => expect(goto).toHaveBeenCalledWith('#/', { replaceState: true }));
		expect(fake.calls.map((c) => c.cmd)).toEqual([['play']]);
	});

	test('Resume when already playing sends nothing', async () => {
		params.action = 'resume';
		fake.app.now = { ...fake.app.now!, mode: 'play' };
		render(GoPage);
		await waitFor(() => expect(goto).toHaveBeenCalled());
		expect(fake.calls).toEqual([]);
	});

	test('Radio starts radio from the current track', async () => {
		params.action = 'radio';
		fake.handler.rpc = async (_p, c) => (c[0] === 'trackinfo' ? MIX_MENU : c[0] === 'dontstopthemusicsetting' ? DSTM_MENU : {});
		render(GoPage);
		await waitFor(() => expect(goto).toHaveBeenCalledWith('#/', { replaceState: true }));
		expect(fake.calls.map((c) => c.cmd).at(-1)).toEqual(['audiomuseai', 'mix', 'track_id:9']);
		fake.handler.rpc = async () => ({});
	});

	test('Radio with nothing to start from goes to Stations and says why', async () => {
		params.action = 'radio';
		fake.app.now = { mode: 'stop', playlist_loop: [] };
		render(GoPage);
		await waitFor(() => expect(goto).toHaveBeenCalledWith('#/stations', { replaceState: true }));
		expect(fake.app.notice).toMatch(/station/);
	});

	test('anything else just opens the app', async () => {
		params.action = 'format-disk';
		render(GoPage);
		await waitFor(() => expect(goto).toHaveBeenCalledWith('#/', { replaceState: true }));
		expect(fake.calls).toEqual([]);
	});
});
