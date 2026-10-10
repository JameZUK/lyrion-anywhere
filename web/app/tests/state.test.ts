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
