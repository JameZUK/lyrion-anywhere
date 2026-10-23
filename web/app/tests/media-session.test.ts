// The Media Session (wave2-design.md §5.5): every action the player supports, seeking
// included, wired to the selected player; the position for the system's progress bar; and
// the handlers taken away again when the component goes.
import { render } from '@testing-library/svelte';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');

type Handler = ((d: MediaSessionActionDetails) => void) | null;
const handlers = new Map<string, Handler>();
const positions: (MediaPositionState | undefined)[] = [];
const fakeSession = {
	metadata: null,
	playbackState: 'none',
	setActionHandler(a: string, fn: Handler) {
		if (a === 'skipad') throw new Error('unsupported');
		handlers.set(a, fn);
	},
	setPositionState(p?: MediaPositionState) {
		positions.push(p);
	},
};
Object.defineProperty(navigator, 'mediaSession', { value: fakeSession, configurable: true });
const { default: MediaSession } = await import('#lib/components/MediaSession.svelte');

const PLAYER = '02:00:00:00:00:01';
const ALL = ['play', 'pause', 'stop', 'nexttrack', 'previoustrack', 'seekto', 'seekbackward', 'seekforward'];
const call = (a: string, d: Partial<MediaSessionActionDetails> = {}) => handlers.get(a)!({ action: a as MediaSessionAction, ...d });

beforeEach(() => {
	handlers.clear();
	positions.length = 0;
	fake.calls.length = 0;
	fake.app.seeks.length = 0;
	fake.app.players = [{ playerid: PLAYER, name: 'Test player' }];
	fake.app.playerId = PLAYER;
	fake.app.elapsed = 100;
	fake.app.now = { mode: 'play', time: 100, duration: 300, playlist_loop: [{ id: 4, title: 'A song', duration: 300 }] };
});
afterAll(() => {
	delete (navigator as { mediaSession?: unknown }).mediaSession;
});

test('every supported action has a handler', () => {
	render(MediaSession);
	for (const a of ALL) expect(typeof handlers.get(a), a).toBe('function');
});

test('transport actions control the selected player', () => {
	render(MediaSession);
	for (const a of ['play', 'pause', 'stop', 'nexttrack', 'previoustrack']) call(a);
	expect(fake.calls.map((c) => c.cmd)).toEqual([['play'], ['pause', 1], ['stop'], ['playlist', 'index', '+1'], ['playlist', 'index', '-1']]);
	expect(fake.calls.every((c) => c.player === PLAYER)).toBe(true);
});

test('seeking: to a point, back and forward (the system’s offset, or 10 s), kept within the track', () => {
	render(MediaSession);
	call('seekto', { seekTime: 42 });
	call('seekbackward', {});
	call('seekforward', { seekOffset: 30 });
	call('seekto', { seekTime: 9999 });
	call('seekto', { seekTime: -5 });
	call('seekto', {});
	// Each seek moves the play head, so back and forward start from the new place.
	expect(fake.app.seeks).toEqual([42, 32, 62, 299, 0]);
});

test('the system’s progress bar gets the position', () => {
	render(MediaSession);
	expect(positions.at(-1)).toEqual({ duration: 300, position: 100, playbackRate: 1 });
});

test('a stream with no length: no seeking offered, and no position', () => {
	fake.app.now = { mode: 'play', time: 5, duration: 0, playlist_loop: [{ id: -1, title: 'Radio' }] };
	render(MediaSession);
	for (const a of ['seekto', 'seekbackward', 'seekforward']) expect(handlers.get(a)).toBeNull();
	expect(typeof handlers.get('play')).toBe('function');
	expect(positions.at(-1)).toBeUndefined();
});

test('a player that refuses seeking: none offered', () => {
	fake.app.now = { ...fake.app.now!, can_seek: 0 };
	render(MediaSession);
	expect(handlers.get('seekto')).toBeNull();
});

test('the handlers are taken away when it goes', () => {
	const r = render(MediaSession);
	r.unmount();
	for (const a of ALL) expect(handlers.get(a), a).toBeNull();
});
