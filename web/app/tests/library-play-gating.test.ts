// Library "play" actions (play, play next, add to queue, playlists, favourites, stations…)
// and the Rooms page need the server: while it is out of reach (or busy) they are disabled,
// visibly and for assistive tech, through one helper (`app.canQueue`), and nothing is sent.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { AlarmListing } from '$lib/api/routine';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('$lib/state/me', () => ({ routine: { alarms: async (): Promise<AlarmListing> => ({ enabled: true, alarms: [], sounds: [] }) } }));
const fake = await import('./helpers/fake-app.svelte');
const { default: AlbumList } = await import('$lib/components/AlbumList.svelte');
const { default: SearchHit } = await import('$lib/components/SearchHit.svelte');
const { default: TrackList } = await import('$lib/components/TrackList.svelte');
const { default: RowMenu } = await import('$lib/components/RowMenu.svelte');
const { default: Rooms } = await import('../src/routes/rooms/+page.svelte');

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
});
afterAll(() => vi.unstubAllGlobals());

const P = '02:00:00:00:00:01';
beforeEach(() => {
	fake.calls.length = 0;
	fake.app.players = [
		{ playerid: P, name: 'Kitchen' },
		{ playerid: '02:00:00:00:00:02', name: 'Lounge' },
	];
	fake.app.playerId = P;
	fake.app.now = { mode: 'play', playlist_tracks: 3 };
	fake.app.reach = 'online';
	fake.handler.rpc = async (_p, c) => (c[0] === 'status' ? { mode: 'play', power: 1, 'mixer volume': 30, playlist_loop: [{ id: 1, title: 'A tune' }] } : {});
});
const sent = () => fake.calls.filter((c) => c.cmd[0] !== 'status' && c.cmd[0] !== 'syncgroups').map((c) => c.cmd);

test('the helper: only while the server answers', () => {
	expect(fake.app.canQueue).toBe(true);
	fake.app.reach = 'busy';
	expect(fake.app.canQueue).toBe(false);
	fake.app.reach = 'unreachable';
	expect(fake.app.canQueue).toBe(false);
});

describe('album lists', () => {
	const albums = [{ id: 7, album: 'Record' }];

	test('in reach: Play works', async () => {
		const r = render(AlbumList, { albums });
		const play = r.getByRole('button', { name: 'Play Record' }) as HTMLButtonElement;
		expect(play.disabled).toBe(false);
		play.click();
		await waitFor(() => expect(sent()[0]?.[0]).toBe('playlistcontrol'));
	});

	test('out of reach: Play disabled, Play next / Add to queue shown unavailable, nothing sent', async () => {
		fake.app.reach = 'unreachable';
		const r = render(AlbumList, { albums });
		const play = r.getByRole('button', { name: 'Play Record' }) as HTMLButtonElement;
		expect(play.disabled).toBe(true);
		(r.getByRole('button', { name: 'More options for Record' }) as HTMLButtonElement).click();
		await tick();
		const items = r.getAllByRole('menuitem');
		expect(items.map((i) => i.getAttribute('aria-disabled'))).toEqual(['true', 'true']);
		for (const i of items) (i as HTMLButtonElement).click();
		await tick();
		expect(sent()).toEqual([]);
	});
});

test('search results: play buttons and menu disabled out of reach', async () => {
	fake.app.reach = 'unreachable';
	const r = render(SearchHit, { hit: { kind: 'track', id: 5, name: 'A song', sub: 'A band', cover: '' } as never });
	for (const b of r.container.querySelectorAll<HTMLButtonElement>('[data-hit-play]')) expect(b.disabled).toBe(true);
	(r.getByRole('button', { name: 'More options for A song' }) as HTMLButtonElement).click();
	await tick();
	for (const i of r.getAllByRole('menuitem')) {
		expect(i.getAttribute('aria-disabled')).toBe('true');
		(i as HTMLButtonElement).click();
	}
	await tick();
	expect(sent()).toEqual([]);
});

test('track lists: rows disabled, menu items unavailable, shortcuts do nothing', async () => {
	const onplay = vi.fn();
	const onnext = vi.fn();
	const onadd = vi.fn();
	const r = render(TrackList, { tracks: [{ id: 1, title: 'One' }], onplay, onnext, onadd, disabled: true });
	const row = r.getByRole('button', { name: 'Play One' }) as HTMLButtonElement;
	expect(row.disabled).toBe(true);
	row.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', bubbles: true }));
	(r.getByRole('button', { name: 'More options for One' }) as HTMLButtonElement).click();
	await tick();
	for (const i of r.getAllByRole('menuitem')) {
		expect(i.getAttribute('aria-disabled')).toBe('true');
		(i as HTMLButtonElement).click();
	}
	expect(onnext).not.toHaveBeenCalled();
	expect(onadd).not.toHaveBeenCalled();
});

test('a row menu item marked unavailable stays focusable but does nothing', async () => {
	const run = vi.fn();
	const r = render(RowMenu, { label: 'Options', actions: [{ label: 'Play next', icon: 'playnext', run, disabled: true }, { label: 'Other', icon: 'more', run: () => {} }] });
	(r.getByRole('button', { name: 'Options' }) as HTMLButtonElement).click();
	await tick();
	const [off, on] = r.getAllByRole('menuitem') as HTMLButtonElement[];
	expect(off!.getAttribute('aria-disabled')).toBe('true');
	expect(off!.disabled).toBe(false); // focusable, per the APG menu pattern
	expect(on!.hasAttribute('aria-disabled')).toBe(false);
	off!.click();
	expect(run).not.toHaveBeenCalled();
});

describe('Rooms', () => {
	test('out of reach: every room is like a remote player: play, volume, power and sync disabled; nothing sent', async () => {
		const r = render(Rooms);
		await waitFor(() => expect(r.container.querySelectorAll('[data-room-play]').length).toBe(2));
		await waitFor(() => expect(r.container.querySelector<HTMLButtonElement>('[data-room-play]')!.disabled).toBe(false));
		fake.app.reach = 'unreachable';
		await tick();
		for (const sel of ['[data-room-play]', '[data-room-power]', '[data-room-grip]', '[data-room-volume]']) {
			for (const el of r.container.querySelectorAll<HTMLButtonElement | HTMLInputElement>(sel)) expect(el.disabled, sel).toBe(true);
		}
		(r.getByRole('button', { name: 'Kitchen options' }) as HTMLButtonElement).click();
		await tick();
		const items = r.getAllByRole('menuitem');
		// Choosing which player to control needs no server; syncing does.
		expect(items.find((i) => i.textContent?.includes('Control'))!.hasAttribute('aria-disabled')).toBe(false);
		expect(items.find((i) => i.textContent?.includes('Sync with'))!.getAttribute('aria-disabled')).toBe('true');
		fake.calls.length = 0;
		for (const i of items) if (i.getAttribute('aria-disabled')) (i as HTMLButtonElement).click();
		const grip = r.container.querySelector<HTMLButtonElement>('[data-room-grip]')!;
		grip.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
		await tick();
		expect(sent()).toEqual([]);
		expect(r.container.querySelector('.error')).toBeNull();
		r.unmount();
	});
});

describe('every library play action uses the helper', () => {
	// A guard for new code: any component or page that queues or plays from the library
	// must disable it with `canQueue` (go/[action] acts on a link, not a control: app.queueUp
	// refuses there by itself).
	const root = `${__dirname}/../src`;
	const files: string[] = [];
	const walk = (d: string) => {
		for (const f of readdirSync(d)) {
			const p = join(d, f);
			if (statSync(p).isDirectory()) walk(p);
			else if (p.endsWith('.svelte')) files.push(p);
		}
	};
	walk(root);
	const PLAYS = /app\.queueUp\(|cmd\.playFavourite\(|activeStation\.start\(/;
	const users = files.filter((f) => PLAYS.test(readFileSync(f, 'utf8')) && !f.includes(`go${'/'}[action]`));

	test('found the call sites', () => {
		expect(users.length).toBeGreaterThanOrEqual(14);
	});

	test.each(users.map((f) => [f.slice(root.length + 1), f]))('%s', (_name, f) => {
		const src = readFileSync(f, 'utf8');
		// Album page: through TrackList's `disabled` too.
		expect(src).toMatch(/canQueue/);
	});
});
