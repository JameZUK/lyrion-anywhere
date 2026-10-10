export {};
// App state across sign-in and sign-out: no timers may outlive the signed-in session.
type Listener = (s: unknown, status: string) => void;
const listeners: Listener[] = [];
vi.mock('$lib/api/session', () => {
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

let release: (() => void) | null = null;
vi.mock('$lib/api/rpc', () => {
	class Api {
		async rpc(_player: string, cmd: unknown[]) {
			if (cmd[0] === 'players') {
				await new Promise<void>((r) => (release = r)); // held until the test lets it go
				return { players_loop: [] };
			}
			return { mode: 'stop' };
		}
		clearArt() {}
	}
	return { Api };
});

const events = { started: 0, stopped: 0 };
vi.mock('$lib/api/events', () => {
	class EventsClient {
		start() {
			events.started++;
		}
		stop() {
			events.stopped++;
		}
	}
	return { EventsClient };
});

await import('$lib/state/app.svelte');
const emit = (status: string) => listeners.forEach((fn) => fn(status === 'signed-in' ? {} : null, status));

afterEach(() => vi.useRealTimers());

// Regression: signing out while the player list was still loading left the polling and
// progress timers running for good (they were created after the load, unconditionally).
test('signing out while the players load leaves no timers behind', async () => {
	vi.useFakeTimers();
	const added = vi.spyOn(document, 'addEventListener');
	emit('signed-in');
	expect(events.started).toBe(1);
	emit('signed-out');
	expect(events.stopped).toBe(1);
	release!();
	await vi.advanceTimersByTimeAsync(0);
	expect(vi.getTimerCount()).toBe(0);
	expect(added.mock.calls.filter(([t]) => t === 'visibilitychange')).toHaveLength(0);
	added.mockRestore();
});

test('out and in again during the load: only the newer start owns the timers', async () => {
	vi.useFakeTimers();
	emit('signed-in');
	const first = release!;
	emit('signed-out');
	emit('signed-in');
	const second = release!;
	first();
	await vi.advanceTimersByTimeAsync(0);
	expect(vi.getTimerCount()).toBe(0); // the stale start created nothing
	second();
	await vi.advanceTimersByTimeAsync(0);
	expect(vi.getTimerCount()).toBe(2); // poll + progress, once
	emit('signed-out');
	expect(vi.getTimerCount()).toBe(0);
});
