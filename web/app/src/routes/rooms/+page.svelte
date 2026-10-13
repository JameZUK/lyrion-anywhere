<script lang="ts">
	// Rooms (docs/plans/wave2-design.md §4.3): every player this user may control, as cards.
	// Tap one to control it. Drag one onto another to play them in sync (the dragged room
	// joins the other's group and takes its queue); the card's "Sync with…" menu item and the
	// keyboard (Enter on the grip, the arrow keys, Enter) do the same. A grid on wide screens,
	// a list on phones. Statuses are read only while this page is open (see RoomsPoller).
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { routine } from '$lib/state/me';
	import { device } from '$lib/player/device.svelte';
	import { bringHere, isThisDevice } from '$lib/player/outputs';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { groupLabel, syncToggle } from '$lib/lms/sync';
	import { MAX_ROOMS, RoomsPoller, alarmText, roomGroup, roomsOf, stepTarget } from '$lib/rooms/rooms.svelte';
	import { alarmChanges } from '$lib/rooms/alarm-changes.svelte';
	import { go } from '$lib/nav';
	import type { Command } from '$lib/api/rpc';
	import type { PlayerInfo } from '$lib/lms/types';
	import type { RowAction } from '$lib/components/RowMenu.svelte';
	import RoomCard from '$lib/components/RoomCard.svelte';
	import Sheet from '$lib/components/Sheet.svelte';
	import { haptics } from '$lib/haptics/haptics.svelte';

	const poller = new RoomsPoller({
		rpc: (p, c) => api.rpc(p, c),
		alarms: (p) => routine.alarms(p),
		visible: () => document.visibilityState !== 'hidden',
	});

	$effect(() => {
		poller.start(() => app.players);
		const unwatch = app.watchEvents((e) => poller.onEvent(e));
		return () => {
			unwatch();
			poller.stop();
		};
	});
	// New players appear (or go): read them at once rather than at the next round.
	let seenPlayers = '';
	$effect(() => {
		const ids = app.players.map((p) => p.playerid).join(',');
		if (seenPlayers && ids !== seenPlayers) untrack(() => void poller.pollAll());
		seenPlayers = ids;
	});
	// Alarms changed in this app: read the next ones again.
	let seenAlarms = alarmChanges.version;
	$effect(() => {
		const v = alarmChanges.version;
		if (v !== seenAlarms) untrack(() => void poller.loadAlarms());
		seenAlarms = v;
	});

	const rooms = $derived(roomsOf(app.players));
	const nameOf = (p: PlayerInfo) => textOf(p.name) || 'Player';
	const key = (p: PlayerInfo) => p.playerid.toLowerCase();
	const byKey = (id: string) => rooms.find((p) => key(p) === id);

	/** What happened, for screen readers (and shown briefly). */
	let notice = $state('');
	let error = $state('');

	async function send(p: PlayerInfo, c: Command, groups = false) {
		error = '';
		try {
			await api.rpc(p.playerid, c);
		} catch (e) {
			error = `${nameOf(p)}: ${(e as Error).message}`;
		}
		poller.soon(key(p), groups);
		if (p.playerid === app.playerId) app.soon();
	}

	function control(p: PlayerInfo) {
		app.select(p.playerid);
		void go('/');
	}
	function playPause(p: PlayerInfo) {
		const s = poller.status[key(p)];
		const playing = Number(s?.power ?? 1) !== 0 && s?.mode === 'play';
		void send(p, playing ? cmd.pause(true) : s?.mode === 'pause' ? cmd.pause(false) : cmd.play());
	}
	function power(p: PlayerInfo) {
		const on = Number(poller.status[key(p)]?.power ?? 1) !== 0;
		void send(p, cmd.power(!on));
	}

	// --- sync -----------------------------------------------------------------------------

	/** `joiner` joins `host`'s group (and takes its queue). */
	async function syncInto(host: string, joiner: string) {
		const h = byKey(host);
		const j = byKey(joiner);
		if (!h || !j || host === joiner) return;
		let t;
		try {
			t = syncToggle(h.playerid, j.playerid, true);
		} catch {
			return; // not a well-formed player id: never sent
		}
		error = '';
		try {
			await api.rpc(t.player, t.cmd);
			notice = `${nameOf(j)} now plays in sync with ${nameOf(h)}.`;
		} catch (e) {
			error = `Couldn't sync ${nameOf(j)} with ${nameOf(h)}: ${(e as Error).message}`;
		}
		poller.soon(host, true);
		poller.soon(joiner);
		app.soon();
	}
	async function unsync(p: PlayerInfo) {
		await send(p, cmd.unsync(), true);
		notice = `${nameOf(p)} now plays on its own.`;
	}

	/** The "Sync with…" chooser for a room. */
	let choosing = $state<PlayerInfo | null>(null);

	// Dragging with a pointer: the grip starts it, the room under the pointer is the target.
	let drag = $state<{ id: string; pointer: number; over: string | null } | null>(null);
	function gripDown(p: PlayerInfo, e: PointerEvent) {
		if (e.button > 0 || rooms.length < 2) return;
		e.preventDefault();
		(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
		drag = { id: key(p), pointer: e.pointerId, over: null };
		haptics.tick('pickup');
	}
	function roomAt(x: number, y: number): string | null {
		const el = typeof document.elementFromPoint === 'function' ? document.elementFromPoint(x, y) : null;
		return el?.closest<HTMLElement>('[data-room]')?.dataset.room ?? null;
	}
	function pointerMove(e: PointerEvent) {
		if (!drag || e.pointerId !== drag.pointer) return;
		const over = roomAt(e.clientX, e.clientY);
		drag = { ...drag, over: over && over !== drag.id ? over : null };
	}
	function pointerUp(e: PointerEvent) {
		if (!drag || e.pointerId !== drag.pointer) return;
		const over = roomAt(e.clientX, e.clientY) ?? drag.over;
		const from = drag.id;
		drag = null;
		haptics.tick('drop');
		if (over && over !== from) void syncInto(over, from);
	}

	// Dragging with the keyboard: Enter or Space on the grip picks the room up, the arrow keys
	// choose where it goes, Enter or Space drops it there, Escape (or leaving) cancels.
	let kb = $state<{ id: string; over: string | null } | null>(null);
	function gripKey(p: PlayerInfo, e: KeyboardEvent) {
		const id = key(p);
		const ids = rooms.map(key);
		if (!kb || kb.id !== id) {
			if (e.key === 'Enter' || e.key === ' ') {
				e.preventDefault();
				if (ids.length < 2) return;
				kb = { id, over: null };
				notice = `${nameOf(p)} picked up. Use the arrow keys to choose a room to sync with, Enter to drop, Escape to cancel.`;
			}
			return;
		}
		if (e.key === 'ArrowDown' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowLeft') {
			e.preventDefault();
			const over = stepTarget(ids, id, kb.over, e.key === 'ArrowDown' || e.key === 'ArrowRight' ? 1 : -1);
			kb = { id, over };
			const o = over ? byKey(over) : null;
			if (o) notice = `Over ${nameOf(o)}.`;
		} else if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			const over = kb.over;
			kb = null;
			if (over) void syncInto(over, id);
			else notice = 'Not moved.';
		} else if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			kb = null;
			notice = `${nameOf(p)} put back.`;
		}
	}

	// --- each card's menu -------------------------------------------------------------------

	/** Something to move: the player in use has a queue. */
	const canMove = $derived(!!app.current && Number(app.now?.playlist_tracks ?? 0) > 0 && !app.moving);

	function actionsFor(p: PlayerInfo): RowAction[] {
		const id = key(p);
		const name = nameOf(p);
		const out: RowAction[] = [{ label: `Control ${name}`, icon: 'speaker', run: () => control(p) }];
		if (rooms.length > 1) out.push({ label: 'Sync with…', icon: 'link', run: () => (choosing = p) });
		if (roomGroup(id, poller.groups).length > 1) out.push({ label: 'Play on its own', icon: 'close', run: () => void unsync(p) });
		if (canMove && p.playerid !== app.playerId) {
			out.push({
				label: 'Move playback here',
				icon: 'transfer',
				run: () => void (isThisDevice(p.playerid) ? bringHere() : app.moveTo(app.playerId, p.playerid, name)),
			});
		}
		return out;
	}

	function note(id: string): string {
		const g = roomGroup(id, poller.groups).filter((x) => x !== id);
		return g.length ? `with ${groupLabel(g, app.players)}` : '';
	}
</script>

<svelte:window onpointermove={pointerMove} onpointerup={pointerUp} onpointercancel={() => (drag = null)} />

<section class="page rooms-page">
	<h1>Rooms</h1>
	<p class="muted help" id="rooms-drag-help">
		Tap a room to control it. Drag a room onto another to play them in sync (or press Enter on its grip, then the arrow keys and Enter). A long press opens its menu.
	</p>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<p class="sr-only" role="status" aria-live="polite" data-rooms-status>{notice}</p>

	{#if !rooms.length}
		<p class="muted">No players you can control.</p>
	{:else}
		<ul class="grid" data-rooms>
			{#each rooms as p (p.playerid)}
				{@const id = key(p)}
				<li>
					<RoomCard
						{id}
						name={nameOf(p)}
						status={poller.status[id]}
						alarm={alarmText(poller.alarms[id])}
						groupNote={note(id)}
						current={p.playerid === app.playerId}
						dragging={drag?.id === id || kb?.id === id}
						target={drag?.over === id || kb?.over === id}
						buttonsOnly={device.buttonsOnly(p.playerid)}
						actions={actionsFor(p)}
						oncontrol={() => control(p)}
						onplaypause={() => playPause(p)}
						onvolume={(v) => void send(p, cmd.volume(v))}
						onpower={() => power(p)}
						ongrip={(e) => gripDown(p, e)}
						ongripkey={(e) => gripKey(p, e)}
					/>
				</li>
			{/each}
		</ul>
		{#if app.players.length > MAX_ROOMS}
			<p class="muted small">Showing the first {MAX_ROOMS} of {app.players.length} players.</p>
		{/if}
	{/if}

	{#if choosing}
		{@const c = choosing}
		<Sheet title={`Sync ${nameOf(c)} with…`} onclose={() => (choosing = null)}>
			<p class="muted small">{nameOf(c)} joins that room's group and plays what it plays.</p>
			<ul class="choose" data-sync-choices>
				{#each rooms.filter((r) => key(r) !== key(c)) as r (r.playerid)}
					<li>
						<button
							class="btn"
							onclick={() => {
								const joiner = key(c);
								choosing = null;
								void syncInto(key(r), joiner);
							}}
							aria-label={`Sync ${nameOf(c)} with ${nameOf(r)}`}>{nameOf(r)}</button
						>
					</li>
				{/each}
			</ul>
		</Sheet>
	{/if}
</section>

<style>
	h1 {
		margin: 0 0 0.25rem;
	}
	.help {
		margin: 0 0 1rem;
		font-size: 0.88rem;
		max-width: 60ch;
	}
	/* A grid on wide screens; one column (a list) on phones. */
	.grid {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.75rem;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr));
	}
	.choose {
		list-style: none;
		margin: 0.5rem 0 0;
		padding: 0;
		display: grid;
		gap: 0.4rem;
	}
	.choose .btn {
		width: 100%;
		justify-content: flex-start;
		min-height: 44px;
	}
</style>
