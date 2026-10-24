import { stubWebAudio } from './helpers/web-audio';
// Player-first start (SPEC D20), in the app state: what is controlled after signing in, the
// fallback when this device can't play, and moving playback.
type Listener = (s: unknown, status: string) => void;
const listeners: Listener[] = [];
vi.mock('#lib/api/session', () => {
	class SessionManager {
		current = null;
		token = 't';
		subscribe(fn: Listener) {
			listeners.push(fn);
			return () => {};
		}
		authed<T>(fn: (t: string) => Promise<T>) {
			return fn('t');
		}
		renew() {
			return Promise.resolve(true);
		}
	}
	return { SessionManager };
});

const ME = '02:4c:41:00:00:01';
const LOUNGE = 'aa:bb:cc:dd:ee:01';
const STUDY = 'aa:bb:cc:dd:ee:02';
let players: { playerid: string; name: string; isplaying?: number }[] = [];
const calls: { player: string; cmd: unknown[] }[] = [];
vi.mock('#lib/api/rpc', () => {
	class Api {
		async rpc(player: string, cmd: unknown[]) {
			calls.push({ player, cmd });
			if (cmd[0] === 'players') return { players_loop: players };
			if (cmd[0] === 'status') return { mode: 'stop', playlist_tracks: 0 };
			return {};
		}
		clearArt() {}
	}
	return { Api };
});
vi.mock('#lib/api/events', () => {
	class EventsClient {
		start() {}
		stop() {}
	}
	return { EventsClient };
});

const { app } = await import('#lib/state/app.svelte');
const session = { user: { username: 'alice', role: 'listener' }, device: { id: 1, name: 'Firefox', mac: ME } };
const signIn = () => listeners.forEach((fn) => fn(session, 'signed-in'));
const signOut = () => listeners.forEach((fn) => fn(null, 'signed-out'));
const settle = () => new Promise((r) => setTimeout(r, 0));

function canPlay(yes: boolean) {
	stubWebAudio(yes);
	vi.stubGlobal('Worker', yes ? class {} : undefined);
}

beforeEach(() => {
	localStorage.clear();
	sessionStorage.clear();
	calls.length = 0;
	players = [
		{ playerid: LOUNGE, name: 'Lounge', isplaying: 1 },
		{ playerid: STUDY, name: 'Study' },
	];
});
afterEach(async () => {
	signOut();
	vi.unstubAllGlobals();
	await settle();
});

test('by default this device is chosen and held until LMS lists it, even with others playing', async () => {
	canPlay(true);
	signIn();
	await settle();
	expect(app.wantDevice).toBe(true);
	expect(app.playerId).toBe(ME);
	expect(app.current, 'not listed yet').toBeUndefined();
	await app.loadPlayers();
	expect(app.playerId, 'not swapped for the playing Lounge').toBe(ME);
	players = [...players, { playerid: ME, name: 'Firefox' }];
	await app.loadPlayers();
	expect(app.current?.playerid).toBe(ME);
	expect(app.remote).toBe(false);
});

test('a named player in Settings is controlled at once; this device does not start', async () => {
	canPlay(true);
	localStorage.setItem('anywhere.defaultPlayer.alice', JSON.stringify({ kind: 'player', id: STUDY }));
	signIn();
	await settle();
	expect(app.wantDevice).toBe(false);
	expect(app.playerId).toBe(STUDY);
	expect(app.remote).toBe(true);
});

test("a browser that can't play controls the last other player, and says why", async () => {
	canPlay(false);
	localStorage.setItem('anywhere.player.remote', STUDY);
	signIn();
	await settle();
	expect(app.wantDevice).toBe(false);
	expect(app.playerId).toBe(STUDY);
	expect(app.notice).toMatch(/can't play music itself/);
});

test("this device's player failing falls back to the last other player", async () => {
	canPlay(true);
	localStorage.setItem('anywhere.player.remote', LOUNGE);
	signIn();
	await settle();
	expect(app.playerId).toBe(ME);
	app.deviceFailed();
	await settle();
	expect(app.playerId).toBe(LOUNGE);
	expect(app.notice).toMatch(/couldn't start/);
	expect(app.wantDevice).toBe(false);
});

test('a reload keeps the remote player the tab was controlling', async () => {
	canPlay(true);
	sessionStorage.setItem('anywhere.player.tab', LOUNGE);
	signIn();
	await settle();
	expect(app.playerId).toBe(LOUNGE);
	expect(app.wantDevice).toBe(false);
});

test('selecting remembers the tab selection and the last other player, never this device as "other"', async () => {
	canPlay(true);
	signIn();
	await settle();
	app.select(STUDY);
	expect(sessionStorage.getItem('anywhere.player.tab')).toBe(STUDY);
	expect(localStorage.getItem('anywhere.player.remote')).toBe(STUDY);
	app.select(ME);
	expect(localStorage.getItem('anywhere.player')).toBe(ME);
	expect(localStorage.getItem('anywhere.player.remote')).toBe(STUDY);
});

test('moving with nothing queued is refused with a note, and nothing is switched', async () => {
	canPlay(true);
	signIn();
	await settle();
	app.select(LOUNGE);
	const ok = await app.moveTo(LOUNGE, STUDY, 'Study');
	expect(ok).toBe(false);
	expect(app.playerId).toBe(LOUNGE);
	expect(app.notice).toMatch(/queue is empty/);
	expect(app.moving).toBe('');
	expect(calls.some((c) => c.cmd[0] === 'sync')).toBe(false);
});

test('signing out clears the note and the wish to start this device', async () => {
	canPlay(false);
	signIn();
	await settle();
	expect(app.notice).not.toBe('');
	signOut();
	expect(app.notice).toBe('');
	expect(app.wantDevice).toBe(false);
});
