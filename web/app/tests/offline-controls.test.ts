// Controls that work (or honestly don't) while the server is out of reach, on every surface:
// the player bar, the queue peek and full queue (with their Downloaded / On this device
// marks), the lock screen, and the one connection message in the layout.
import { readFileSync } from 'node:fs';
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { PlayerStatus, Track } from '#lib/lms/types';
import type { StoredTrack } from '#lib/offline/types';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { default: PlayerBar } = await import('#lib/components/PlayerBar.svelte');
const { default: QueueList } = await import('#lib/components/QueueList.svelte');
const { default: QueuePage } = await import('../src/routes/queue/+page.svelte');
const { device } = await import('#lib/player/device.svelte');
const { controls, local } = await import('#lib/player/controls.svelte');
const { downloads } = await import('#lib/offline/index.svelte');

type Handler = ((d: MediaSessionActionDetails) => void) | null;
const handlers = new Map<string, Handler>();
Object.defineProperty(navigator, 'mediaSession', {
	value: { metadata: null, playbackState: 'none', setActionHandler: (a: string, fn: Handler) => handlers.set(a, fn), setPositionState() {} },
	configurable: true,
});
const { default: MediaSession } = await import('#lib/components/MediaSession.svelte');

const P = '02:00:00:00:00:01';
const track = (n: number): Track => ({ id: n, title: `Song ${n}`, artist: 'Invented', duration: 180 });
const queue = [1, 2, 3, 4, 5].map(track);
const stored = (id: number): StoredTrack => ({
	user: 'u', key: `k${id}`, id, title: `Song ${id}`, artist: '', album: '', albumId: null, tracknum: null, duration: 180,
	format: 'mp3', codec: 'mp3', ext: 'mp3', size: 1, contentType: 'audio/mpeg', downloadedAt: 0, chunks: 1,
});

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
	takeRecords() {
		return [];
	}
}
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
	// The element downloads play through: this browser plays MP3.
	vi.spyOn(HTMLMediaElement.prototype, 'canPlayType').mockImplementation((t: string) => (t.includes('mpeg') ? 'maybe' : ''));
});
afterAll(() => {
	vi.unstubAllGlobals();
	delete (navigator as { mediaSession?: unknown }).mediaSession;
});

const status = (): PlayerStatus => ({ mode: 'play', time: 30, duration: 180, playlist_cur_index: 1, playlist_tracks: queue.length, 'mixer volume': 50, playlist_loop: [queue[1]!, queue[2]!] });

beforeEach(() => {
	fake.calls.length = 0;
	handlers.clear();
	fake.app.players = [{ playerid: P, name: 'Test player' }];
	fake.app.playerId = P;
	fake.app.now = status();
	fake.app.elapsed = 30;
	fake.app.reach = 'online';
	device.mac = '';
	device.detail = {};
	downloads.saved = [stored(3)];
	local.sync(false, true);
	local.rows = new Map(queue.map((t, i) => [i, t]));
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'status' && cmd[1] !== '-') {
			const from = Number(cmd[1]);
			return { playlist_loop: queue.slice(from, from + Number(cmd[2])), playlist_tracks: queue.length, playlist_cur_index: 1 };
		}
		return {};
	};
});

/** This device is the player, its stream playing, and the server out of reach. */
function thisDeviceOutOfReach() {
	device.mac = P;
	device.detail = { output: 'Playing', received: true };
	fake.app.reach = 'unreachable';
	local.sync(true, false);
}

const button = (c: HTMLElement, name: string) => c.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!;
const settle = async () => {
	for (let i = 0; i < 5; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

describe('the player bar', () => {
	test('in reach: everything works, as before', async () => {
		const r = render(PlayerBar);
		await tick();
		for (const n of ['Previous', 'Pause', 'Next']) expect(button(r.container, n).disabled, n).toBe(false);
		r.unmount();
	});

	test('a remote player out of reach: transport and volume disabled (and say so to assistive tech)', async () => {
		fake.app.reach = 'unreachable';
		const r = render(PlayerBar);
		await tick();
		for (const n of ['Previous', 'Pause', 'Next']) expect(button(r.container, n).disabled, n).toBe(true);
		expect(r.container.querySelector<HTMLInputElement>('input[aria-label="Position"]')!.disabled).toBe(true);
		expect(r.container.querySelector<HTMLInputElement>('.volume input')?.disabled ?? true).toBe(true);
		button(r.container, 'Next').click();
		await settle();
		expect(fake.calls).toEqual([]);
		r.unmount();
	});

	test('busy gates like out of reach', async () => {
		fake.app.reach = 'busy';
		const r = render(PlayerBar);
		await tick();
		expect(button(r.container, 'Pause').disabled).toBe(true);
		r.unmount();
	});

	test('this device out of reach: Pause works here; Next only to a track that is here', async () => {
		thisDeviceOutOfReach();
		const r = render(PlayerBar);
		await tick();
		expect(button(r.container, 'Pause').disabled).toBe(false);
		expect(button(r.container, 'Next').disabled).toBe(false); // track 3 is downloaded
		expect(button(r.container, 'Previous').disabled).toBe(true); // track 1 isn't
		const sent: unknown[] = [];
		const post = vi.spyOn(device, 'local').mockImplementation((m) => void sent.push(m));
		button(r.container, 'Pause').click();
		await tick();
		expect(sent).toEqual([{ hold: true }]);
		expect(fake.calls).toEqual([]); // nothing to the server
		post.mockRestore();
		r.unmount();
	});
});

describe('the queue peek', () => {
	test('marks: Downloaded for any player, with an accessible label', async () => {
		const r = render(QueueList);
		await waitFor(() => expect(r.container.querySelectorAll('li').length).toBe(4));
		const row = r.container.querySelector<HTMLButtonElement>('button[data-mark="downloaded"]')!;
		expect(row.getAttribute('aria-label')).toBe('Play Song 3 by Invented. Downloaded');
		expect(row.querySelector('[data-queue-mark="downloaded"]')?.getAttribute('title')).toBe('Downloaded');
		expect(r.container.querySelectorAll('[data-queue-mark]').length).toBe(1);
		r.unmount();
	});

	test('On this device: the stream this device holds, only while it is the player and only out of reach', async () => {
		device.mac = P;
		device.detail = { output: 'Playing', received: true };
		// In reach, the cache is invisible (Wave 3.5 rule 2): nothing to mark.
		const online = render(QueueList);
		await waitFor(() => expect(online.container.querySelectorAll('li').length).toBe(4));
		expect(online.container.querySelector('[data-queue-mark="device"]')).toBeNull();
		online.unmount();
		fake.app.reach = 'unreachable';
		const r = render(QueueList);
		await waitFor(() => expect(r.container.querySelectorAll('li').length).toBe(4));
		const now = r.container.querySelector<HTMLButtonElement>('button[aria-current="true"]')!;
		expect(now.dataset.mark).toBe('device');
		expect(now.getAttribute('aria-label')).toMatch(/\. On this device$/);
		r.unmount();
		fake.app.playerId = '02:00:00:00:00:09';
		fake.app.players = [{ playerid: '02:00:00:00:00:09', name: 'Kitchen' }];
		const r2 = render(QueueList);
		await waitFor(() => expect(r2.container.querySelectorAll('li').length).toBe(4));
		expect(r2.container.querySelector('[data-queue-mark="device"]')).toBeNull();
		r2.unmount();
	});

	test('out of reach: rows that can’t play here are dimmed and not tappable', async () => {
		const r = render(QueueList);
		await waitFor(() => expect(r.container.querySelectorAll('li').length).toBe(4));
		thisDeviceOutOfReach();
		await tick();
		const rows = [...r.container.querySelectorAll('li')];
		expect(rows.map((li) => li.classList.contains('unavailable'))).toEqual([false, false, true, true]);
		expect(rows.map((li) => li.querySelector('button')!.disabled)).toEqual([false, false, true, true]);
		r.unmount();
	});

	test('out of reach: a failed refresh shows no error of its own', async () => {
		fake.app.reach = 'unreachable';
		fake.handler.rpc = async () => {
			throw new TypeError('Failed to fetch');
		};
		const r = render(QueueList);
		await settle();
		expect(r.container.querySelector('.error')).toBeNull();
		r.unmount();
	});
});

describe('the full queue', () => {
	test('out of reach: no edits; only rows this device can play are tappable; marks shown', async () => {
		const r = render(QueuePage);
		await waitFor(() => expect(r.container.querySelectorAll('li[data-index]').length).toBe(5));
		thisDeviceOutOfReach();
		await tick();
		const lis = [...r.container.querySelectorAll<HTMLLIElement>('li[data-index]')];
		expect(lis.map((li) => li.classList.contains('unavailable'))).toEqual([true, false, false, true, true]);
		expect(lis[2]!.querySelector('[data-queue-mark="downloaded"]')).not.toBeNull();
		expect(lis[1]!.querySelector('[data-queue-mark="device"]')).not.toBeNull();
		// Every edit in a row's menu (and the queue's own menu) is unavailable, and says why.
		await fireEvent.click(r.getByRole('button', { name: 'More options for Song 2' }));
		const items = r.getAllByRole('menuitem');
		expect(items.map((m) => m.textContent?.replace(/(N|Delete|Alt\+Arrow\w+)$/, ''))).toEqual(['Play next', 'Move up', 'Move down', 'Remove from queue']);
		for (const m of items) expect(m.getAttribute('aria-disabled')).toBe('true');
		r.unmount();
	});
});

describe('the lock screen', () => {
	test('a remote player out of reach: no actions offered', async () => {
		fake.app.reach = 'unreachable';
		const r = render(MediaSession);
		await tick();
		for (const a of ['play', 'pause', 'stop', 'nexttrack', 'previoustrack', 'seekto']) expect(handlers.get(a), a).toBeNull();
		r.unmount();
	});

	test('this device out of reach: only what works here', async () => {
		thisDeviceOutOfReach();
		const r = render(MediaSession);
		await tick();
		for (const a of ['play', 'pause', 'nexttrack']) expect(typeof handlers.get(a), a).toBe('function');
		for (const a of ['stop', 'previoustrack', 'seekto', 'seekforward', 'seekbackward']) expect(handlers.get(a), a).toBeNull();
		r.unmount();
	});

	test('back in reach: everything again', async () => {
		thisDeviceOutOfReach();
		const r = render(MediaSession);
		await tick();
		fake.app.reach = 'online';
		local.sync(false, true);
		await tick();
		for (const a of ['play', 'pause', 'stop', 'nexttrack', 'previoustrack', 'seekto']) expect(typeof handlers.get(a), a).toBe('function');
		r.unmount();
	});
});

describe('the connection message', () => {
	const layout = readFileSync(`${__dirname}/../src/routes/+layout.svelte`, 'utf8');

	test('one status slot, in the content (every width), before the page; no sidebar-only note', () => {
		const content = layout.slice(layout.indexOf('<main class="content"'));
		expect(content.indexOf('<StatusSlot />')).toBeLessThan(content.indexOf('{@render children()}'));
		expect((layout.match(/<StatusSlot \/>/g) ?? []).length).toBe(2); // the shell, and car mode
		const nav = layout.slice(layout.indexOf('<nav class="side"'), layout.indexOf('</nav>'));
		expect(nav).not.toMatch(/StatusSlot|chip/);
	});

	test('it takes room in the layout: never fixed or floating over the controls', () => {
		const css = layout.slice(layout.indexOf('.chip {'), layout.indexOf('}', layout.indexOf('.chip {')));
		expect(css).not.toMatch(/position:\s*(fixed|absolute|sticky)/);
		expect(css).not.toMatch(/z-index/);
		// Not hidden on phones.
		expect(layout).not.toMatch(/\.chip\s*\{[^}]*display:\s*none/);
	});

	test('wording follows what plays', () => {
		fake.app.reach = 'online';
		expect(controls.message).toBe('');
		fake.app.reach = 'busy';
		expect(controls.message).toBe('Your music server is busy. Waiting for it…');
		fake.app.reach = 'unreachable';
		expect(controls.message).toBe('Can’t reach your music server.');
		thisDeviceOutOfReach();
		expect(controls.message).toBe('Can’t reach your music server. Playing from this device.');
	});
});
