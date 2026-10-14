<script lang="ts">
	// "Play on…": where the music plays (SPEC D20). This device first (ticked when in use), then
	// the other players this user may control, with what each is playing. Wave 3.5: tapping a
	// row plays on that player from now on; moving the playback there (Transfer) is the row's
	// secondary action, in its ⋯ ("Move playback to Kitchen"). Grouping is one "Group
	// players…" row, which opens the sync switches. On phones it carries the volume too.
	import { untrack } from 'svelte';
	import { api, app, session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import { bringHere, isThisDevice, useThisDevice } from '$lib/player/outputs';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { groupFromStatus, groupLabel, groupOf, syncGroupsFrom, syncToggle } from '$lib/lms/sync';
	import type { PlayerInfo, PlayerStatus, SyncGroupsResult } from '$lib/lms/types';
	import Icon from './Icon.svelte';
	import VolumeSlider from './VolumeSlider.svelte';
	import RowMenu, { type RowAction } from './RowMenu.svelte';
	import { PlayerManager } from '$lib/rooms/manager.svelte';
	import { go } from '$lib/nav';
	import Switch from './Switch.svelte';
	import { controls } from '$lib/player/controls.svelte';

	// `volume`: the selected player's volume at the top (phones: Now playing has no volume row).
	let { onclose, volume = false }: { onclose: () => void; volume?: boolean } = $props();
	/** The player list, or the sync switches ("Group players…"). */
	let view = $state<'list' | 'group'>('list');
	/** Moving playback and grouping need the server. */
	const offline = $derived(app.reach !== 'online');

	// Focus moves into the panel when it opens, and back to whatever opened it when it closes;
	// Escape closes it.
	let sheet = $state<HTMLDivElement>();
	$effect(() => {
		const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		sheet?.focus();
		return () => {
			if (opener?.isConnected) opener.focus();
		};
	});
	// A tap or click anywhere outside the panel closes it, like any menu. Not the button that
	// opened it (marked aria-expanded): that toggles it itself.
	function onpointerdown(e: PointerEvent) {
		const t = e.target as Element | null;
		if (!sheet || !t || sheet.contains(t) || t.closest?.('[aria-expanded="true"]')) return;
		onclose();
	}
	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && !e.defaultPrevented) {
			e.preventDefault();
			onclose();
		}
	}

	const selected = $derived(app.playerId.toLowerCase());
	const hereSelected = $derived(isThisDevice(selected));
	/** The players other than this device. */
	const others = $derived(app.players.filter((p) => !isThisDevice(p.playerid)));
	/** Something to move: the player in use has a queue. */
	const canMove = $derived(!!app.current && Number(app.now?.playlist_tracks ?? 0) > 0 && !app.moving);

	function control(id: string) {
		app.select(id);
		onclose();
	}
	function here() {
		useThisDevice();
		onclose();
	}
	function bring() {
		onclose();
		void bringHere();
	}
	/** This device's ⋯: move the playback here; its player's alarms, once LMS lists it. */
	const hereActions = $derived.by((): RowAction[] => {
		const out: RowAction[] = [];
		if (!hereSelected && device.supported && canMove) out.push({ label: 'Move playback to this device', icon: 'transfer', run: bring, disabled: offline, why: controls.why(offline), id: 'move' });
		const mine = app.players.find((p) => isThisDevice(p.playerid));
		if (mine)
			out.push({
				label: 'Alarms',
				icon: 'alarm',
				run: () => {
					onclose();
					app.select(mine.playerid);
					void go('/alarms');
				},
				id: 'alarms',
			});
		return out;
	});

	// --- what each player is playing, and its menu: the one player manager (Wave 3.5) -------

	const manager = new PlayerManager({
		rpc: (p, c) => api.rpc(p, c),
		// Play on… reads the groups itself (for its Group players… view), and no alarm times.
		groups: false,
		visible: () => document.visibilityState !== 'hidden',
		app,
		bringHere,
		isThisDevice,
		why: (off) => controls.why(off),
	});
	const playing = $derived(Object.fromEntries(others.map((p) => [p.playerid, manager.status(p)]).filter(([, s]) => !!s).map(([id, s]) => [id, describe(s as PlayerStatus)])) as Record<string, string>);
	function describe(s: PlayerStatus): string {
		if (Number(s.power ?? 1) === 0) return 'Off';
		const t = s.playlist_loop?.[0];
		const what = [textOf(t?.title ?? s.current_title), textOf(t?.artist)].filter(Boolean).join(' · ');
		if (s.mode === 'play') return what ? `Playing ${what}` : 'Playing';
		if (s.mode === 'pause') return what ? `Paused: ${what}` : 'Paused';
		return what ? `Stopped: ${what}` : 'Idle';
	}
	/** A player's ⋯: move playback there (closing the sheet), play/pause it, its alarms, power. */
	function rowActions(p: PlayerInfo): RowAction[] {
		return manager
			.actionsFor(p, {
				alarms: () => {
					app.select(p.playerid);
					void go('/alarms');
				},
			})
			.map((a) => (a.id === 'move' || a.id === 'alarms' ? { ...a, run: () => (onclose(), a.run()) } : a));
	}

	// --- sync groups ----------------------------------------------------------------------
	// Only players listed in app.players (the ones this user may control) appear or are sent.

	let groups = $state<string[][]>([]);
	let syncBusy = $state(false);
	let syncError = $state('');

	async function loadGroups() {
		if (app.players.length < 2) return;
		try {
			const r = await api.rpc<SyncGroupsResult>('', cmd.syncGroups());
			groups = syncGroupsFrom(r, app.players);
			syncError = '';
		} catch (e) {
			syncError = (e as Error).message;
		}
	}
	$effect(() => {
		void app.players;
		untrack(() => void loadGroups());
	});
	// Every player's status while the sheet is open (and none once it closes).
	$effect(() => {
		manager.start(() => others);
		return () => manager.stop();
	});

	/** The selected player's group: from `syncgroups ?`, else from its own status. */
	const myGroup = $derived.by(() => {
		const g = groupOf(selected, groups);
		return g.length ? g : groupFromStatus(selected, app.now, app.players);
	});
	const syncable = $derived(app.players.filter((p) => p.playerid.toLowerCase() !== selected));
	const nameOf = (id: string) => textOf(app.players.find((p) => p.playerid.toLowerCase() === id)?.name);

	/** "with Study", for a player in a group, naming the others. */
	function groupNote(id: string): string {
		const g = groupOf(id, groups);
		const rest = g.filter((x) => x !== id.toLowerCase());
		return rest.length ? `with ${groupLabel(rest, app.players)}` : '';
	}

	async function runSync(player: string, c: (string | number)[]) {
		syncBusy = true;
		try {
			await api.rpc(player, c);
			syncError = '';
		} catch (e) {
			syncError = `Couldn't change the group: ${(e as Error).message}`;
		} finally {
			syncBusy = false;
			await loadGroups();
			app.soon();
		}
	}

	function toggle(other: string, on: boolean) {
		if (!selected || syncBusy) return;
		let t;
		try {
			t = syncToggle(selected, other.toLowerCase(), on);
		} catch {
			return; // not a well-formed player id: never sent
		}
		void runSync(t.player, t.cmd);
	}

	function leave() {
		if (!selected || syncBusy) return;
		void runSync(selected, cmd.unsync());
	}

	const deviceSub = $derived.by(() => {
		if (!device.supported) return "This browser can't play music itself";
		const name = textOf(session.current?.device.name);
		const state = {
			off: 'Ready',
			starting: 'Starting…',
			connected: hereSelected && app.playing ? 'Playing' : 'Ready',
			reconnecting: 'Reconnecting…',
			stopped: 'Ready',
			error: "Couldn't start; tap to try again",
		}[device.state];
		return [name, state].filter(Boolean).join(' · ');
	});
</script>

<svelte:window {onkeydown} {onpointerdown} />

<div class="sheet" role="dialog" aria-label={view === 'group' ? 'Group players' : 'Play on'} tabindex="-1" bind:this={sheet}>
	<header>
		{#if view === 'group'}
			<button class="icon-btn" onclick={() => (view = 'list')} aria-label="Back to Play on"><Icon name="back" /></button>
		{/if}
		<h2>{view === 'group' ? 'Group players' : 'Play on'}</h2>
		<button class="icon-btn" onclick={onclose} aria-label="Close"><Icon name="close" /></button>
	</header>

	{#if view === 'list'}
		{#if volume && app.playerId}<VolumeSlider disabled={!controls.gates.volume} />{/if}

		<ul class="outputs">
			<li class="output" class:selected={hereSelected}>
				<button
					class="row choice"
					aria-current={hereSelected ? 'true' : undefined}
					aria-label={`Play on this device${hereSelected ? ' (in use)' : ''}`}
					disabled={!device.supported}
					onclick={here}
					data-output="device"
				>
					<Icon name="device" />
					<span class="grow">
						<span class="name">This device</span>
						<span class="sub muted ellipsis">{deviceSub}</span>
					</span>
					{#if hereSelected}<span class="tick"><Icon name="check" size={20} label="In use" /></span>{/if}
				</button>
				{#if hereActions.length}<RowMenu label="More options for this device" actions={hereActions} />{/if}
			</li>

			{#each others as p (p.playerid)}
				{@const on = p.playerid.toLowerCase() === selected}
				{@const name = textOf(p.name)}
				{@const acts = rowActions(p)}
				<li class="output" class:selected={on}>
					<button class="row choice" aria-current={on ? 'true' : undefined} onclick={() => control(p.playerid)} aria-label={`Play on ${name}${on ? ' (in use)' : ''}`} data-output={p.playerid}>
						<Icon name="speaker" />
						<span class="grow">
							<span class="name ellipsis">{name}</span>
							<span class="sub muted ellipsis">
								{playing[p.playerid] ?? (p.isplaying ? 'Playing' : p.power ? 'On' : 'Off')}{groupNote(p.playerid) ? ` · ${groupNote(p.playerid)}` : ''}
							</span>
						</span>
						{#if p.isplaying}<span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>{/if}
						{#if on}<span class="tick"><Icon name="check" size={20} label="In use" /></span>{/if}
					</button>
					{#if acts.length}
						<!-- One trailing control (Wave 3.5 rule 3): what Rooms' card had is in the row's ⋯. -->
						<RowMenu label={`More options for ${name}`} actions={acts} />
					{/if}
				</li>
			{/each}
			{#if !others.length}<li class="muted empty">No other players you can control.</li>{/if}
		</ul>
		{#if selected && app.current && syncable.length}
			<button class="row choice group-row" onclick={() => (view = 'group')} data-group-players>
				<Icon name="link" />
				<span class="grow">
					<span class="name">Group players…</span>
					<span class="sub muted ellipsis">{myGroup.length > 1 ? `${nameOf(selected) || 'This player'} ${groupNote(selected)}` : 'Play in sync on several players'}</span>
				</span>
				<span class="chev" aria-hidden="true">›</span>
			</button>
		{/if}
		{#if app.players.length > 1}
			<!-- The expanded view, on tablets and desktops; on a phone this sheet has everything. -->
			<a class="row choice rooms" href="#/rooms" onclick={onclose} data-rooms-link><Icon name="rooms" /><span class="grow"><span class="name">Rooms</span><span class="sub muted">Every player at a glance</span></span><span class="chev" aria-hidden="true">›</span></a>
		{/if}
	{:else}
		<section class="sync" aria-labelledby="sync-h">
			<h3 id="sync-h">Play {nameOf(selected) || 'this player'} in sync with…</h3>
			{#if syncError}<p class="error" role="alert">{syncError}</p>{/if}
			<ul>
				{#each syncable as p (p.playerid)}
					{@const on = myGroup.includes(p.playerid.toLowerCase())}
					<li>
						<Switch
							checked={on}
							onchange={(v) => toggle(p.playerid, v)}
							label={textOf(p.name)}
							note={!on && groupNote(p.playerid) ? `Grouped ${groupNote(p.playerid)}` : ''}
							aria-label={`Sync with ${textOf(p.name)}`}
							disabled={syncBusy || offline}
						>
							{#snippet icon()}<Icon name={isThisDevice(p.playerid) ? 'device' : 'speaker'} size={18} />{/snippet}
						</Switch>
					</li>
				{/each}
			</ul>
			{#if myGroup.length > 1}
				<button class="btn leave" disabled={syncBusy || offline} onclick={leave}><Icon name="unlink" size={18} />Play {nameOf(selected) || 'this player'} on its own</button>
			{/if}
		</section>
	{/if}
</div>

<style>
	.sheet {
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius);
		box-shadow: var(--shadow);
		width: min(92vw, 380px);
		max-height: min(70vh, 600px);
		overflow: auto;
		padding: 0.5rem;
		text-align: left;
	}
	.sheet:focus {
		outline: none; /* the panel itself only holds focus for keyboard users to move on from */
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.25rem 0.25rem 0.25rem 0.75rem;
	}
	h2 {
		margin: 0;
		font-size: 1rem;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.output {
		display: flex;
		align-items: center;
		gap: 0.25rem;
		border-radius: var(--radius-sm);
		padding-right: 0.4rem;
	}
	.output:hover {
		background: var(--bg-sunken);
	}
	.output.selected {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		min-width: 0;
		padding: 0.6rem 0.75rem;
		border-radius: var(--radius-sm);
		text-align: left;
		color: inherit;
	}
	.output .row {
		flex: 1;
	}
	.choice {
		border: 0;
		background: transparent;
	}
	.choice:disabled {
		opacity: 0.6;
		cursor: default;
	}
	.grow {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.name {
		font-weight: 550;
	}
	.sub {
		font-size: 0.82rem;
	}
	.selected .sub {
		color: inherit;
		opacity: 0.8;
	}
	.tick {
		display: inline-grid;
		color: var(--accent);
	}
	.group-row,
	.rooms {
		margin-top: 0.15rem;
	}
	@media (max-width: 760px) {
		.rooms {
			display: none;
		}
	}
	.group-row:hover,
	.rooms:hover {
		background: var(--bg-sunken);
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.2rem;
	}
	.empty {
		padding: 0.75rem;
	}
	.sync {
		padding-top: 0.2rem;
	}
	h3 {
		margin: 0.25rem 0.75rem 0.4rem;
		font-size: 0.85rem;
		font-weight: 600;
		color: var(--fg-muted);
	}
	.sync .error {
		margin: 0.25rem 0.75rem;
	}
	.leave {
		margin: 0.4rem 0.5rem 0.25rem;
	}
	.eq {
		display: inline-flex;
		gap: 2px;
		align-items: flex-end;
		height: 14px;
	}
	.eq i {
		width: 3px;
		background: var(--accent);
		animation: eq 0.9s ease-in-out infinite;
	}
	.eq i:nth-child(2) {
		animation-delay: -0.3s;
	}
	.eq i:nth-child(3) {
		animation-delay: -0.6s;
	}
	@keyframes eq {
		0%,
		100% {
			height: 30%;
		}
		50% {
			height: 100%;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.eq i {
			animation: none;
			height: 70%;
		}
	}
</style>
