<script lang="ts">
	// Pick which player to control; start or stop this browser as a player.
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { groupFromStatus, groupLabel, groupOf, syncGroupsFrom, syncToggle } from '$lib/lms/sync';
	import type { SyncGroupsResult } from '$lib/lms/types';
	import Icon from './Icon.svelte';

	let { onclose }: { onclose: () => void } = $props();

	// Focus moves into the panel when it opens, and back to whatever opened it (the player
	// button) when it closes; Escape closes it.
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

	function pick(id: string) {
		app.select(id);
		onclose();
	}

	async function startHere() {
		await device.start();
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

	const selected = $derived(app.playerId.toLowerCase());
	/** The selected player's group: from `syncgroups ?`, else from its own status. */
	const myGroup = $derived.by(() => {
		const g = groupOf(selected, groups);
		return g.length ? g : groupFromStatus(selected, app.now, app.players);
	});
	const others = $derived(app.players.filter((p) => p.playerid.toLowerCase() !== selected));
	const nameOf = (id: string) => textOf(app.players.find((p) => p.playerid.toLowerCase() === id)?.name);

	/** "With Study", for a player in a group, naming the others. */
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
			t = syncToggle(selected, other.toLowerCase(), groups.length ? groups : myGroup.length ? [myGroup] : [], on);
		} catch {
			return; // not a well-formed player id: never sent
		}
		void runSync(t.player, t.cmd);
	}

	function leave() {
		if (!selected || syncBusy) return;
		void runSync(selected, cmd.unsync());
	}

	const deviceLabel = $derived(
		{
			off: 'Play on this device',
			starting: 'Starting…',
			connected: 'Playing here',
			reconnecting: 'Reconnecting…',
			stopped: 'Play on this device',
			error: 'Could not start; try again',
		}[device.state],
	);
</script>

<svelte:window {onkeydown} {onpointerdown} />

<div class="sheet" role="dialog" aria-label="Choose a player" tabindex="-1" bind:this={sheet}>
	<header>
		<h2>Players</h2>
		<button class="icon-btn" onclick={onclose} aria-label="Close"><Icon name="close" /></button>
	</header>

	<div class="here">
		{#if device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting'}
			<div class="row active-device">
				<Icon name="device" />
				<span class="grow">{deviceLabel}</span>
				<button class="btn" onclick={() => device.stop()}>Stop</button>
			</div>
		{:else if device.supported}
			<button class="row choice" onclick={startHere}>
				<Icon name="device" />
				<span class="grow">{deviceLabel}</span>
				<Icon name="plus" size={18} />
			</button>
		{:else}
			<p class="muted">This browser can't play here (no AudioWorklet).</p>
		{/if}
	</div>

	<ul>
		{#each app.players as p (p.playerid)}
			<li>
				<button class="row choice" class:selected={p.playerid === app.playerId} onclick={() => pick(p.playerid)}>
					<Icon name={p.playerid === device.mac ? 'device' : 'speaker'} />
					<span class="grow">
						<span class="name ellipsis">{textOf(p.name)}</span>
						<span class="sub muted">
							{p.isplaying ? 'Playing' : p.power ? 'On' : 'Off'}{p.playerid === device.mac ? ' · this device' : ''}{groupNote(p.playerid) ? ` · ${groupNote(p.playerid)}` : ''}
						</span>
					</span>
					{#if p.isplaying}<span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>{/if}
				</button>
			</li>
		{:else}
			<li class="muted empty">No players you can control yet.</li>
		{/each}
	</ul>

	{#if selected && others.length}
		<section class="sync" aria-labelledby="sync-h">
			<h3 id="sync-h">Play {nameOf(selected) || 'this player'} in sync with…</h3>
			{#if syncError}<p class="error" role="alert">{syncError}</p>{/if}
			<ul>
				{#each others as p (p.playerid)}
					{@const on = myGroup.includes(p.playerid.toLowerCase())}
					<li>
						<button
							class="row choice toggle"
							role="switch"
							aria-checked={on}
							aria-label={`Sync with ${textOf(p.name)}`}
							disabled={syncBusy}
							onclick={() => toggle(p.playerid, !on)}
						>
							<Icon name="speaker" size={18} />
							<span class="grow">
								<span class="name ellipsis">{textOf(p.name)}</span>
								{#if !on && groupNote(p.playerid)}<span class="sub muted ellipsis">Grouped {groupNote(p.playerid)}</span>{/if}
							</span>
							<span class="track" class:on aria-hidden="true"><span class="knob"></span></span>
						</button>
					</li>
				{/each}
			</ul>
			{#if myGroup.length > 1}
				<button class="btn leave" disabled={syncBusy} onclick={leave}>Play {nameOf(selected) || 'this player'} on its own</button>
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
		width: min(92vw, 360px);
		max-height: min(70vh, 560px);
		overflow: auto;
		padding: 0.5rem;
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
	.row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		padding: 0.6rem 0.75rem;
		border-radius: var(--radius-sm);
		text-align: left;
	}
	.choice {
		border: 0;
		background: transparent;
	}
	.choice:hover {
		background: var(--bg-sunken);
	}
	.selected {
		background: var(--accent-soft);
	}
	.active-device {
		background: var(--accent-soft);
	}
	.here {
		border-bottom: 1px solid var(--line);
		padding-bottom: 0.4rem;
		margin-bottom: 0.4rem;
	}
	.here p {
		padding: 0.5rem 0.75rem;
		margin: 0;
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
	.empty {
		padding: 0.75rem;
	}
	.sync {
		border-top: 1px solid var(--line);
		margin-top: 0.4rem;
		padding-top: 0.4rem;
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
	.toggle {
		min-height: 48px;
	}
	.track {
		flex: none;
		position: relative;
		width: 38px;
		height: 22px;
		border-radius: 11px;
		background: var(--line);
		transition: background 0.15s;
	}
	.track.on {
		background: var(--accent);
	}
	.knob {
		position: absolute;
		top: 3px;
		left: 3px;
		width: 16px;
		height: 16px;
		border-radius: 50%;
		background: var(--bg-raised);
		transition: transform 0.15s;
	}
	.track.on .knob {
		transform: translateX(16px);
	}
	.leave {
		margin: 0.4rem 0.75rem 0.25rem;
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
		.track,
		.knob {
			transition: none;
		}
		.eq i {
			animation: none;
			height: 70%;
		}
	}
</style>
