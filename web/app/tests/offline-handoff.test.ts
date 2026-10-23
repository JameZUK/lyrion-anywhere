// "Continue on <player>" (SPEC §8.4): the offline queue handed to an LMS player. Keys are
// checked against what the gateway says each id names now (unresolvable tracks skipped and
// counted), the queue goes in order with the current track, position and play state, and a
// failure anywhere leaves the offline player alone. The offer appears only with a queue,
// only after being offline, and only to the user whose downloads they are.
import {
	HandoffOffer,
	MAX_HANDOFF,
	continueOn,
	defaultChoice,
	handoffChoices,
	handoffCommands,
	handoffText,
	planHandoff,
	resolveTracks,
	snapshotOf,
	type Lookup,
	type QueueSnapshot,
} from '#lib/offline/handoff.svelte';
import type { Command } from '#lib/api/rpc';
import { ApiError } from '#lib/api/http';
import { key } from './helpers/fake-downloads';

const K = (n: number) => key(0xa000 + n);
const PLAYER = 'aa:bb:cc:dd:ee:01';

/** The library now: id → key. Ids not here are unknown (404). */
function library(entries: Record<number, string>, asked: number[] = []): Lookup {
	return async (id) => {
		asked.push(id);
		return id in entries ? { id, key: entries[id]! } : null;
	};
}

function snap(n: number, over: Partial<QueueSnapshot> = {}): QueueSnapshot {
	return { user: 'alice', tracks: Array.from({ length: n }, (_, i) => ({ id: i + 1, key: K(i + 1) })), index: 0, position: 0, playing: true, ...over };
}

const all = (n: number) => Object.fromEntries(Array.from({ length: n }, (_, i) => [i + 1, K(i + 1)]));

function rig(s: () => QueueSnapshot | null, look: Lookup, fail?: (c: Command, n: number) => boolean) {
	const sent: Command[] = [];
	const stopLocal = vi.fn();
	const rpc = vi.fn(async (p: string, c: Command) => {
		expect(p).toBe(PLAYER);
		if (fail?.(c, sent.length)) throw new Error('network error');
		sent.push(c);
		return {};
	});
	return { sent, stopLocal, deps: { current: s, lookup: look, rpc, stopLocal, now: () => 0 } };
}

const ids = (sent: Command[]) =>
	sent
		.filter((c) => c[0] === 'playlistcontrol')
		.flatMap((c) =>
			String(c[2])
				.replace('track_id:', '')
				.split(',')
				.map(Number),
		);

describe('resolving keys', () => {
	test('only ids that still name the download’s file; each id asked once', async () => {
		const asked: number[] = [];
		const look = library({ 1: K(1), 2: K(99), 3: K(3) }, asked);
		const got = await resolveTracks(
			[
				{ id: 1, key: K(1) },
				{ id: 2, key: K(2) }, // renumbered: another file now
				{ id: 4, key: K(4) }, // gone
				{ id: 3, key: K(3) },
				{ id: 1, key: K(1) },
			],
			look,
		);
		expect(got).toEqual([1, null, null, 3, 1]);
		expect(asked.sort()).toEqual([1, 2, 3, 4]);
	});

	test('malformed ids and keys are never sent', async () => {
		const asked: number[] = [];
		const got = await resolveTracks(
			[
				{ id: 0, key: K(0) },
				{ id: -1, key: K(1) },
				{ id: 1.5, key: K(1) },
				{ id: 1e13, key: K(1) },
				{ id: 2, key: '../../etc/passwd' },
				{ id: 3, key: K(3).toUpperCase() },
			],
			library(all(5), asked),
		);
		expect(got.every((x) => x === null)).toBe(true);
		expect(asked).toEqual([]);
	});

	test('an answer for another id confirms nothing', async () => {
		const got = await resolveTracks([{ id: 1, key: K(1) }], async () => ({ id: 2, key: K(1) }));
		expect(got).toEqual([null]);
	});

	test('any other failure throws (nothing half-checked)', async () => {
		const look: Lookup = async (id) => {
			if (id === 2) throw new ApiError(403, 'downloads are not allowed for this account');
			return { id, key: K(id) };
		};
		await expect(resolveTracks(snap(3).tracks, look)).rejects.toThrow('not allowed');
	});
});

describe('the plan', () => {
	test('keeps order, starts at the current track and position, skips the unresolved', () => {
		const s = snap(5, { index: 2, position: 73.4, playing: true });
		const p = planHandoff(s, [1, null, 3, 4, null])!;
		expect(p).toEqual({ ids: [1, 3, 4], start: 1, seek: 73.4, paused: false, skipped: 2 });
	});

	test('a skipped current track: the next one, from its start; else the last before it', () => {
		expect(planHandoff(snap(4, { index: 1, position: 50 }), [1, null, 3, 4])).toMatchObject({ ids: [1, 3, 4], start: 1, seek: 0 });
		expect(planHandoff(snap(3, { index: 2, position: 50 }), [1, 2, null])).toMatchObject({ ids: [1, 2], start: 1, seek: 0 });
		expect(planHandoff(snap(2), [null, null])).toBeNull();
	});

	test('paused stays paused; playing plays', () => {
		const paused = handoffCommands(planHandoff(snap(2, { index: 1, position: 30, playing: false }), [1, 2])!);
		expect(paused).toEqual([
			['power', 1],
			['playlist', 'shuffle', 0],
			['playlistcontrol', 'cmd:load', 'track_id:1,2', 'play_index:1'],
			['time', 30],
			['pause', 1],
		]);
		const playing = handoffCommands(planHandoff(snap(2, { index: 1, position: 30 }), [1, 2])!);
		expect(playing.some((c) => c[0] === 'pause')).toBe(false);
		// A second in is as good as the start.
		expect(handoffCommands(planHandoff(snap(1, { position: 1.2 }), [1])!).some((c) => c[0] === 'time')).toBe(false);
	});

	test('a long queue goes in batches that fit the gateway’s limit, in order, then a jump', () => {
		const n = 300;
		const s = { ...snap(n, { index: 250, position: 10 }), tracks: Array.from({ length: n }, (_, i) => ({ id: 100_000 + i, key: K(i) })) };
		const p = planHandoff(
			s,
			s.tracks.map((t) => t.id),
		)!;
		const c = handoffCommands(p);
		const loads = c.filter((x) => x[0] === 'playlistcontrol');
		expect(loads.length).toBeGreaterThan(2);
		expect(loads[0]![1]).toBe('cmd:load');
		expect(loads.slice(1).every((x) => x[1] === 'cmd:add')).toBe(true);
		expect(loads.every((x) => String(x[2]).length - 'track_id:'.length <= 200)).toBe(true);
		expect(ids(c)).toEqual(s.tracks.map((t) => t.id));
		expect(c).toContainEqual(['playlist', 'index', 250]);
	});

	test('a very long queue keeps the stretch around the current track', () => {
		const n = MAX_HANDOFF + 500;
		const s = snap(n, { index: n - 10 });
		const p = planHandoff(
			s,
			s.tracks.map((t) => t.id),
		)!;
		expect(p.ids.length).toBe(MAX_HANDOFF);
		expect(p.ids[p.start]).toBe(n - 9);
		expect(p.ids.at(-1)).toBe(n);
		expect(p.skipped).toBe(500);
	});
});

describe('continuing', () => {
	test('loads the resolved queue in order, seeks, then stops the local player', async () => {
		const s = snap(4, { index: 2, position: 61, playing: true });
		const { sent, stopLocal, deps } = rig(() => s, library({ 1: K(1), 3: K(3), 4: K(4) }));
		const r = await continueOn(PLAYER, deps);
		expect(r).toEqual({ ok: true, moved: 3, skipped: 1 });
		expect(sent).toEqual([
			['power', 1],
			['playlist', 'shuffle', 0],
			['playlistcontrol', 'cmd:load', 'track_id:1,3,4', 'play_index:1'],
			['time', 61],
		]);
		expect(stopLocal).toHaveBeenCalledTimes(1);
		expect(handoffText(r, 'Kitchen')).toBe('Now playing on Kitchen. 1 track was skipped: it isn’t in the library any more.');
	});

	test('playing: the seek allows for the time the hand-off took', async () => {
		let t = 0;
		const s = snap(1, { position: 30 });
		const { sent, deps } = rig(() => s, library(all(1)));
		deps.now = () => (t += 1500);
		await continueOn(PLAYER, deps);
		expect(sent.find((c) => c[0] === 'time')![1]).toBeGreaterThan(30);
	});

	test('a failure part-way leaves the local player untouched', async () => {
		const s = snap(3, { index: 1, position: 40 });
		for (const at of [0, 2, 3]) {
			const { stopLocal, deps } = rig(
				() => s,
				library(all(3)),
				(_c, n) => n === at,
			);
			const r = await continueOn(PLAYER, deps);
			expect(r).toEqual({ ok: false, reason: 'error', message: 'network error' });
			expect(stopLocal).not.toHaveBeenCalled();
			expect(handoffText(r, 'Kitchen')).toContain('Your downloads carry on here.');
		}
	});

	test('a refused lookup (signed out, no permission, offline) sends nothing', async () => {
		const look: Lookup = async () => {
			throw new ApiError(401, 'unauthorised');
		};
		const { sent, stopLocal, deps } = rig(() => snap(2), look);
		expect(await continueOn(PLAYER, deps)).toMatchObject({ ok: false, reason: 'error' });
		expect(sent).toEqual([]);
		expect(stopLocal).not.toHaveBeenCalled();
	});

	test('nothing resolvable: nothing sent, the local player keeps going', async () => {
		const { sent, stopLocal, deps } = rig(() => snap(2), library({}));
		expect(await continueOn(PLAYER, deps)).toEqual({ ok: false, reason: 'none' });
		expect(sent).toEqual([]);
		expect(stopLocal).not.toHaveBeenCalled();
	});

	test('an empty queue, or a malformed player: nothing sent', async () => {
		const a = rig(() => null, library(all(2)));
		expect(await continueOn(PLAYER, a.deps)).toEqual({ ok: false, reason: 'empty' });
		const b = rig(() => snap(2), library(all(2)));
		expect(await continueOn('../x', b.deps)).toMatchObject({ ok: false, reason: 'error' });
		expect([...a.sent, ...b.sent]).toEqual([]);
	});

	test('the queue changed while it was checked: the new queue goes, with its position', async () => {
		let s = snap(2);
		const asked: number[] = [];
		const base = library(all(5), asked);
		const look: Lookup = async (id) => {
			const r = await base(id);
			// The user picks another album while the first is checked.
			s = { ...snap(5), tracks: [3, 4, 5].map((i) => ({ id: i, key: K(i) })), index: 1, position: 12, playing: false };
			return r;
		};
		const { sent, deps } = rig(() => s, look);
		const r = await continueOn(PLAYER, deps);
		expect(r).toEqual({ ok: true, moved: 3, skipped: 0 });
		expect(ids(sent)).toEqual([3, 4, 5]);
		expect(sent).toContainEqual(['time', 12]);
		expect(sent.at(-1)).toEqual(['pause', 1]);
	});

	test('another user’s queue is never continued', async () => {
		let n = 0;
		const { sent, deps } = rig(() => (n++ === 0 ? snap(2) : snap(2, { user: 'bob' })), library(all(2)));
		expect(await continueOn(PLAYER, deps)).toEqual({ ok: false, reason: 'changed' });
		expect(sent).toEqual([]);
	});
});

describe('the offer', () => {
	test('only after being offline, only with a queue, and once', () => {
		const o = new HandoffOffer();
		o.backOnline('alice', snap(2));
		expect(o.offered).toBeNull(); // never offline: nothing to offer
		o.wentOffline();
		o.backOnline('alice', null);
		expect(o.offered).toBeNull(); // nothing queued
		o.wentOffline();
		o.backOnline('alice', snap(2));
		expect(o.offered).toBe('alice');
		o.dismiss();
		o.backOnline('alice', snap(2));
		expect(o.offered).toBeNull(); // "Not now" sticks until the next time offline
	});

	test('follows the live queue, and goes when it empties', () => {
		const o = new HandoffOffer();
		o.wentOffline();
		o.backOnline('alice', snap(2));
		expect(o.queue(snap(3))?.tracks.length).toBe(3);
		expect(o.queue(null)).toBeNull();
	});

	test('a queue stopped by an ended sign-in is offered to the same user only, paused', () => {
		const o = new HandoffOffer();
		o.wentOffline();
		o.keep(snap(3, { index: 1, position: 20, playing: true }));
		o.backOnline('bob', null);
		expect(o.offered).toBeNull();
		expect(o.kept('alice')).toBeNull(); // forgotten once someone else signed in

		const p = new HandoffOffer();
		p.wentOffline();
		p.keep(snap(3, { index: 1, position: 20, playing: true }));
		p.backOnline('alice', null);
		expect(p.offered).toBe('alice');
		expect(p.queue(null)).toMatchObject({ index: 1, position: 20, playing: false });
		expect(p.queue(snap(1, { user: 'bob' }))?.user).toBe('alice');
		p.reset();
		expect(p.queue(null)).toBeNull();
	});

	test('the snapshot of the offline player', () => {
		expect(snapshotOf({ queue: [], index: -1, position: 0, playing: false, user: 'alice' })).toBeNull();
		expect(snapshotOf({ queue: [{ id: 1, key: K(1) }], index: 0, position: 5, playing: true, user: '' })).toBeNull();
		expect(snapshotOf({ queue: [{ id: 1, key: K(1) }], index: 0, position: NaN, playing: true, user: 'alice' })).toEqual({
			user: 'alice',
			tracks: [{ id: 1, key: K(1) }],
			index: 0,
			position: 0,
			playing: true,
		});
	});

	test('the players offered: this device first, then the listed ones; the last used by default', () => {
		const players = [
			{ playerid: PLAYER, name: 'Kitchen' },
			{ playerid: '02:00:00:00:00:01', name: 'This browser' },
			{ playerid: 'aa:bb:cc:dd:ee:02', name: '' },
		];
		const c = handoffChoices(players, { mac: '02:00:00:00:00:01', supported: true });
		expect(c.map((x) => [x.id, x.name, x.device])).toEqual([
			['02:00:00:00:00:01', 'This device', true],
			[PLAYER, 'Kitchen', false],
			['aa:bb:cc:dd:ee:02', 'aa:bb:cc:dd:ee:02', false],
		]);
		expect(defaultChoice(c, PLAYER)).toBe(PLAYER);
		expect(defaultChoice(c, '02:00:00:00:00:01')).toBe('02:00:00:00:00:01');
		expect(defaultChoice(c, '')).toBe('02:00:00:00:00:01');
		// A player that isn't listed (not this user's) is never the default.
		expect(defaultChoice(c, 'aa:bb:cc:dd:ee:99')).toBe('02:00:00:00:00:01');
		const noDevice = handoffChoices(players, { mac: '', supported: false });
		expect(noDevice.some((x) => x.device)).toBe(false);
		expect(defaultChoice(noDevice, '')).toBe(PLAYER);
		expect(defaultChoice([], '')).toBe('');
	});
});
