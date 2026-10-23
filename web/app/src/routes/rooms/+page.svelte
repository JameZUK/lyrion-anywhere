<script lang="ts">
	// Rooms (docs/plans/wave2-design.md §4.3; Wave 3.5 item 13): every player this user may
	// control, as cards: the expanded view of Play on… on tablets and desktops (the two share
	// one player manager, lib/rooms/manager.svelte.ts). Tap one to play on it. Drag one by its
	// cover onto another to play them in sync (the dragged room joins the other's group and
	// takes its queue); the card's "Sync with…" menu item does the same from the keyboard. A
	// grid on wide screens, a list on phones. Statuses are read only while this page is open.
	import { untrack } from 'svelte';
	import { api, app } from '#lib/state/app.svelte';
	import { routine } from '#lib/state/me';
	import { device } from '#lib/player/device.svelte';
	import { bringHere, isThisDevice } from '#lib/player/outputs';
	import { MAX_ROOMS, alarmText, roomsOf } from '#lib/rooms/rooms.svelte';
	import { PlayerManager, keyOf, nameOf } from '#lib/rooms/manager.svelte';
	import { alarmChanges } from '#lib/rooms/alarm-changes.svelte';
	import { go } from '#lib/nav';
	import type { PlayerInfo } from '#lib/lms/types';
	import RoomCard from '#lib/components/RoomCard.svelte';
	import Sheet from '#lib/components/Sheet.svelte';
	import { haptics } from '#lib/haptics/haptics.svelte';

	const players = new PlayerManager({
		rpc: (p, c) => api.rpc(p, c),
		alarms: (p) => routine.alarms(p),
		visible: () => document.visibilityState !== 'hidden',
		app,
		bringHere,
		isThisDevice,
	});
	const poller = players.poller;

	$effect(() => {
		players.start(() => app.players);
		const unwatch = app.watchEvents((e) => poller.onEvent(e));
		return () => {
			unwatch();
			players.stop();
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

	function control(p: PlayerInfo) {
		app.select(p.playerid);
		void go('/');
	}
	function alarmsOf(p: PlayerInfo) {
		app.select(p.playerid);
		void go('/alarms');
	}

	/** The "Sync with…" chooser for a room. */
	let choosing = $state<PlayerInfo | null>(null);

	// Dragging with a pointer: a card's cover starts it, the room under the pointer is the target.
	let drag = $state<{ id: string; pointer: number; over: string | null } | null>(null);
	function dragDown(p: PlayerInfo, e: PointerEvent) {
		if (e.button > 0 || rooms.length < 2 || !app.canQueue) return;
		e.preventDefault();
		(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
		drag = { id: keyOf(p), pointer: e.pointerId, over: null };
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
		if (over && over !== from) void players.syncInto(over, from);
	}

	/** Each card's ⋯: play on it, sync with another, then the manager's items. */
	function actionsFor(p: PlayerInfo) {
		const off = !app.canQueue;
		return players.actionsFor(p, {
			extra: [
				{ label: `Play on ${nameOf(p)}`, icon: 'speaker', run: () => control(p), id: 'play-on' },
				...(rooms.length > 1 ? [{ label: 'Sync with…', icon: 'link', run: () => (choosing = p), disabled: off, id: 'sync' }] : []),
			],
			alarms: () => alarmsOf(p),
		});
	}
</script>

<svelte:window onpointermove={pointerMove} onpointerup={pointerUp} onpointercancel={() => (drag = null)} />

<section class="page rooms-page">
	<h1>Rooms</h1>
	{#if !app.canQueue && rooms.length}
		<p class="muted">Rooms will update when your music server is back.</p>
	{/if}
	{#if players.error}<p class="error" role="alert">{players.error}</p>{/if}
	<p class="sr-only" role="status" aria-live="polite" data-rooms-status>{players.notice}</p>

	{#if !rooms.length}
		<p class="muted">No players you can control.</p>
	{:else}
		<ul class="grid" data-rooms>
			{#each rooms as p (p.playerid)}
				{@const id = keyOf(p)}
				<li>
					<RoomCard
						{id}
						name={nameOf(p)}
						status={poller.status[id]}
						alarm={alarmText(poller.alarms[id])}
						groupNote={players.groupNote(p)}
						current={p.playerid === app.playerId}
						dragging={drag?.id === id}
						target={drag?.over === id}
						buttonsOnly={device.buttonsOnly(p.playerid)}
						unavailable={!app.canQueue}
						actions={actionsFor(p)}
						oncontrol={() => control(p)}
						onplaypause={() => players.playPause(p)}
						onvolume={(v) => players.volume(p, v)}
						onpower={() => players.power(p)}
						ondrag={(e) => dragDown(p, e)}
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
				{#each rooms.filter((r) => keyOf(r) !== keyOf(c)) as r (r.playerid)}
					<li>
						<button
							class="btn"
							onclick={() => {
								const joiner = keyOf(c);
								choosing = null;
								void players.syncInto(keyOf(r), joiner);
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
		margin: 0 0 0.75rem;
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
