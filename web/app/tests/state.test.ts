export {};
// App state: a remembered player that no longer exists must not be polled.
vi.mock('$lib/api/session', () => {
	class SessionManager {
		current = null;
		token = 't';
		subscribe() {
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

const calls: { player: string; cmd: unknown[] }[] = [];
let players: { playerid: string; name: string; isplaying?: number }[] = [];
vi.mock('$lib/api/rpc', () => {
	class Api {
		async rpc(player: string, cmd: unknown[]) {
			calls.push({ player, cmd });
			if (cmd[0] === 'players') return { players_loop: players };
			return { mode: 'stop' };
		}
		art() {
			return Promise.resolve('blob:x');
		}
		clearArt() {}
	}
	return { Api };
});

const { app } = await import('$lib/state/app.svelte');

beforeEach(() => {
	calls.length = 0;
	localStorage.clear();
});

test('a remembered player that has gone is forgotten and never polled', async () => {
	players = [];
	app.playerId = 'aa:bb:cc:dd:ee:ff';
	localStorage.setItem('anywhere.player', 'aa:bb:cc:dd:ee:ff');
	await app.loadPlayers();
	expect(app.playerId).toBe('');
	expect(localStorage.getItem('anywhere.player')).toBeNull();
	await app.refresh();
	await app.send(['pause']);
	expect(calls.filter((c) => c.player !== '')).toEqual([]);
});

test('an existing player is selected and polled', async () => {
	players = [{ playerid: '02:4c:41:00:00:09', name: 'Mine' }];
	app.playerId = 'aa:bb:cc:dd:ee:ff'; // stale
	await app.loadPlayers();
	expect(app.playerId).toBe('02:4c:41:00:00:09');
	await app.refresh();
	expect(calls.some((c) => c.player === '02:4c:41:00:00:09' && c.cmd[0] === 'status')).toBe(true);
});

// Regression (Android): the device connected before LMS listed its player, so the app picked
// another player, or none, and never selected this device.
test('a just-connected device is selected once LMS lists it', async () => {
	players = [{ playerid: 'aa:bb:cc:dd:ee:01', name: 'Kitchen' }];
	app.playerId = '';
	const device = { playerid: '02:4c:41:00:00:07', name: 'Firefox on Android' };
	let loads = 0;
	const realPlayers = players;
	// The device appears on the third look.
	const ok = app.waitForPlayer(device.playerid, 10, 1);
	const timer = setInterval(() => {
		loads++;
		if (loads === 2) players = [...realPlayers, device];
	}, 1);
	expect(await ok).toBe(true);
	clearInterval(timer);
	expect(app.playerId).toBe(device.playerid);
	// Later refreshes keep it.
	await app.loadPlayers();
	expect(app.playerId).toBe(device.playerid);
});

test('waiting gives up quietly if the player never appears', async () => {
	players = [];
	expect(await app.waitForPlayer('02:4c:41:00:00:08', 3, 1)).toBe(false);
});
