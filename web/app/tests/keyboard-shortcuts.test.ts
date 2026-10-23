// Keyboard shortcuts (wave2-design.md §5.5): Space, the arrow keys (on Now playing and in car
// mode), `/` for search, `L` for lyrics and `?` for the help sheet. They never fire while
// typing, in a dialog or menu, with a modifier, or on a control that uses the key itself.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { goto } from '$app/navigation';
import { shortcutFor, type KeyLike } from '#lib/keys/shortcuts';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const url = { hash: '#/' };
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return new URL(`http://app.example/${url.hash}`);
		},
		params: {},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { nowPlaying } = await import('#lib/nowplaying/store.svelte');
const { keysHelp } = await import('#lib/keys/help.svelte');
const { default: KeyboardShortcuts } = await import('#lib/components/KeyboardShortcuts.svelte');

const PLAYER = '02:00:00:00:00:01';

/** A fixture element from fixed test markup, added to the page. */
function el(markup: string): HTMLElement {
	const parsed = new DOMParser().parseFromString(`<body>${markup}</body>`, 'text/html').body.firstElementChild!;
	const node = document.importNode(parsed, true) as HTMLElement;
	document.body.append(node);
	return node;
}
const key = (k: string, target: EventTarget | null = document.body, more: Partial<KeyLike> = {}): KeyLike => ({ key: k, target, ...more });

afterEach(() => {
	document.body.replaceChildren();
	keysHelp.open = false;
});

describe('which key does what', () => {
	const on = { arrows: true };
	test('each key', () => {
		expect(shortcutFor(key(' '), on)).toBe('playpause');
		expect(shortcutFor(key('ArrowLeft'), on)).toBe('seekback');
		expect(shortcutFor(key('ArrowRight'), on)).toBe('seekforward');
		expect(shortcutFor(key('ArrowLeft', document.body, { shiftKey: true }), on)).toBe('previous');
		expect(shortcutFor(key('ArrowRight', document.body, { shiftKey: true }), on)).toBe('next');
		expect(shortcutFor(key('ArrowUp'), on)).toBe('volumeup');
		expect(shortcutFor(key('ArrowDown'), on)).toBe('volumedown');
		expect(shortcutFor(key('/'), on)).toBe('search');
		expect(shortcutFor(key('l'), on)).toBe('lyrics');
		expect(shortcutFor(key('L', document.body, { shiftKey: true }), on)).toBe('lyrics');
		expect(shortcutFor(key('?', document.body, { shiftKey: true }), on)).toBe('help');
		expect(shortcutFor(key('x'), on)).toBeNull();
		expect(shortcutFor(key('Enter'), on)).toBeNull();
	});

	test('the arrows act only where the page doesn’t scroll', () => {
		for (const k of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown']) expect(shortcutFor(key(k), { arrows: false })).toBeNull();
		expect(shortcutFor(key(' '), { arrows: false })).toBe('playpause');
		expect(shortcutFor(key('/'), { arrows: false })).toBe('search');
	});

	test('never while typing', () => {
		for (const markup of ['<input type="text" />', '<input type="search" />', '<textarea></textarea>', '<select><option>a</option></select>', '<div contenteditable="true"><span>x</span></div>']) {
			const t = el(markup);
			const target = t.querySelector('span') ?? t;
			for (const k of [' ', '/', 'l', '?', 'ArrowLeft', 'ArrowUp']) expect(shortcutFor(key(k, target), on)).toBeNull();
		}
	});

	test('never in a dialog or menu, or while a modal dialog is open', () => {
		for (const markup of ['<div role="dialog"><p>t</p></div>', '<div role="menu"><p>t</p></div>', '<dialog><p>t</p></dialog>', '<div aria-modal="true"><p>t</p></div>']) {
			const t = el(markup).querySelector('p')!;
			for (const k of [' ', '/', 'l', '?']) expect(shortcutFor(key(k, t), on)).toBeNull();
		}
		document.body.replaceChildren();
		el('<dialog open><p>sheet</p></dialog>');
		expect(shortcutFor(key(' '), on)).toBeNull();
		expect(shortcutFor(key('?'), on)).toBeNull();
	});

	test('never with Ctrl, ⌘ or Alt, while composing, when already handled, or held down', () => {
		for (const mod of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { isComposing: true }, { defaultPrevented: true }]) {
			for (const k of [' ', '/', 'l', '?', 'ArrowLeft']) expect(shortcutFor(key(k, document.body, mod), on)).toBeNull();
		}
		for (const k of [' ', '/', 'l', '?']) expect(shortcutFor(key(k, document.body, { repeat: true }), on)).toBeNull();
		// Holding ↑ keeps turning the volume up.
		expect(shortcutFor(key('ArrowUp', document.body, { repeat: true }), on)).toBe('volumeup');
	});

	test('Space and the arrows stay with controls that use them', () => {
		for (const markup of ['<button>b</button>', '<a href="#/x">a</a>', '<div role="slider" tabindex="0"></div>', '<div role="tab" tabindex="0"></div>', '<summary>s</summary>']) {
			const t = el(markup);
			for (const k of [' ', 'ArrowLeft', 'ArrowUp']) expect(shortcutFor(key(k, t), on)).toBeNull();
			// …but `/`, `L` and `?` still work from them.
			expect(shortcutFor(key('/', t), on)).toBe('search');
			expect(shortcutFor(key('l', t), on)).toBe('lyrics');
		}
	});

	test('robustness: odd targets and keys never throw', () => {
		for (const target of [null, window, document, {} as EventTarget, document.createTextNode('x') as unknown as EventTarget]) {
			for (const k of [' ', '', 'Dead', 'Unidentified', '\u0000', 'ArrowUp']) expect(() => shortcutFor(key(k, target), on)).not.toThrow();
		}
	});
});

describe('the shortcuts in the app', () => {
	beforeEach(() => {
		url.hash = '#/';
		fake.calls.length = 0;
		fake.app.seeks.length = 0;
		fake.app.players = [{ playerid: PLAYER, name: 'Test player' }];
		fake.app.playerId = PLAYER;
		fake.app.elapsed = 60;
		fake.app.now = { mode: 'play', time: 60, duration: 240, 'mixer volume': 40, playlist_loop: [{ id: 3, title: 'A song' }] };
		nowPlaying.setLayout('art');
		vi.mocked(goto).mockClear();
	});
	const press = (k: string, more: KeyboardEventInit = {}) => fireEvent.keyDown(document.body, { key: k, ...more });
	const sent = () => fake.calls.map((c) => c.cmd);

	test('Space pauses and plays, and says so', async () => {
		render(KeyboardShortcuts);
		await press(' ');
		expect(sent()).toEqual([['pause', 1]]);
		await waitFor(() => expect(document.querySelector('[data-shortcut-said]')!.textContent).toBe('Paused'));
		fake.app.now = { ...fake.app.now!, mode: 'pause' };
		await press(' ');
		expect(sent()[1]).toEqual(['play']);
	});

	test('arrows on Now playing: seek, skip and volume', async () => {
		render(KeyboardShortcuts);
		await press('ArrowRight');
		await press('ArrowLeft');
		expect(fake.app.seeks).toEqual([70, 60]);
		await press('ArrowRight', { shiftKey: true });
		await press('ArrowLeft', { shiftKey: true });
		await press('ArrowUp');
		await press('ArrowDown');
		expect(sent()).toEqual([
			['playlist', 'index', '+1'],
			['playlist', 'index', '-1'],
			['mixer', 'volume', 45],
			['mixer', 'volume', 35],
		]);
	});

	test('arrows elsewhere are left to the page (scrolling)', async () => {
		url.hash = '#/albums';
		render(KeyboardShortcuts);
		await press('ArrowUp');
		await press('ArrowRight');
		expect(sent()).toEqual([]);
		expect(fake.app.seeks).toEqual([]);
	});

	test('a stream without a length can’t be seeked', async () => {
		fake.app.now = { ...fake.app.now!, duration: 0 };
		render(KeyboardShortcuts);
		await press('ArrowRight');
		expect(fake.app.seeks).toEqual([]);
	});

	test('/ goes to search and puts the cursor in the box', async () => {
		url.hash = '#/albums';
		render(KeyboardShortcuts);
		const box = el('<input data-search-input type="search" />');
		await press('/');
		await waitFor(() => expect(goto).toHaveBeenCalledWith('#/search'));
		await waitFor(() => expect(document.activeElement).toBe(box));
	});

	test('L shows the lyrics on Now playing, and again goes back to the art', async () => {
		url.hash = '#/queue';
		render(KeyboardShortcuts);
		await press('l');
		expect(nowPlaying.layout).toBe('lyrics');
		await waitFor(() => expect(goto).toHaveBeenCalledWith('#/'));
		url.hash = '#/';
		await press('L', { shiftKey: true });
		expect(nowPlaying.layout).toBe('art');
	});

	test('? opens the help sheet listing every shortcut; nothing else fires while it is open', async () => {
		render(KeyboardShortcuts);
		await press('?', { shiftKey: true });
		const sheet = await screen.findByRole('dialog', { name: 'Keyboard shortcuts' });
		const text = sheet.textContent!;
		for (const label of ['Play or pause', 'Search', 'Lyrics', 'Volume up or down', 'Previous or next track', 'This list of shortcuts', 'Art, Lyrics or Info layout']) {
			expect(text).toContain(label);
		}
		expect(sheet.querySelectorAll('kbd').length).toBeGreaterThan(8);
		await press(' ');
		await press('/');
		expect(sent()).toEqual([]);
		expect(goto).not.toHaveBeenCalled();
		await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(keysHelp.open).toBe(false);
	});

	test('typing in a field never triggers anything', async () => {
		render(KeyboardShortcuts);
		const box = el('<input type="text" />');
		box.focus();
		for (const k of [' ', '/', 'l', '?', 'ArrowUp']) await fireEvent.keyDown(box, { key: k });
		expect(sent()).toEqual([]);
		expect(goto).not.toHaveBeenCalled();
		expect(keysHelp.open).toBe(false);
	});
});
