<script lang="ts">
	// One room on the Rooms page (Wave 2 §4.3): what it plays, play/pause, volume, power,
	// sleep remaining and the next alarm. One accessible group, its controls labelled with the
	// room's name ("Kitchen volume"). Tapping the card controls that player; a long press (or
	// its menu) offers Sync with…, Play on its own and Move playback here. The grip drags it
	// onto another room to sync, by pointer or keyboard (the page runs the drag).
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { formatLeft, sleepLeft } from '$lib/lms/sleep';
	import type { PlayerStatus } from '$lib/lms/types';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import RowMenu, { type RowAction } from './RowMenu.svelte';

	let {
		id,
		name,
		status,
		alarm = '',
		groupNote = '',
		current = false,
		dragging = false,
		target = false,
		buttonsOnly = false,
		actions,
		oncontrol,
		onplaypause,
		onvolume,
		onpower,
		ongrip,
		ongripkey,
	}: {
		id: string;
		name: string;
		status: PlayerStatus | undefined;
		/** "tomorrow 07:00", or '' for none. */
		alarm?: string;
		/** "With Lounge", when it plays in a group. */
		groupNote?: string;
		current?: boolean;
		dragging?: boolean;
		target?: boolean;
		/** This device's own player can't have its volume set here (see device.buttonsOnly). */
		buttonsOnly?: boolean;
		actions: readonly RowAction[];
		oncontrol: () => void;
		onplaypause: () => void;
		onvolume: (v: number) => void;
		onpower: () => void;
		ongrip: (e: PointerEvent) => void;
		ongripkey: (e: KeyboardEvent) => void;
	} = $props();

	const LONG_PRESS_MS = 550;

	const track = $derived(status?.playlist_loop?.[0]);
	const on = $derived(Number(status?.power ?? 1) !== 0);
	const playing = $derived(on && status?.mode === 'play');
	const title = $derived(textOf(track?.title ?? status?.current_title));
	const artist = $derived(textOf(track?.artist));
	const sleep = $derived(sleepLeft(status));
	const mode = $derived(!status ? '' : !on ? 'Off' : playing ? 'Playing' : status.mode === 'pause' ? 'Paused' : 'Stopped');

	/** The slider's own value while it moves; LMS's otherwise. */
	let moving = $state<number | null>(null);
	let timer: ReturnType<typeof setTimeout> | undefined;
	const volume = $derived(moving ?? (Math.abs(Number(status?.['mixer volume'] ?? 0)) || 0));
	function slide(v: number) {
		moving = v;
		clearTimeout(timer);
		timer = setTimeout(() => {
			onvolume(v);
			moving = null;
		}, 150);
	}

	let menu = $state<ReturnType<typeof RowMenu>>();
	let press: { id: number; x: number; y: number; timer: ReturnType<typeof setTimeout> } | null = null;
	let swallow = false;
	function down(e: PointerEvent) {
		if (e.button > 0) return;
		const p = { id: e.pointerId, x: e.clientX, y: e.clientY, timer: setTimeout(() => {
			swallow = true;
			press = null;
			menu?.show();
		}, LONG_PRESS_MS) };
		press = p;
	}
	function move(e: PointerEvent) {
		if (press && e.pointerId === press.id && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 10) cancel();
	}
	function cancel() {
		if (press) clearTimeout(press.timer);
		press = null;
	}
	function click(e: MouseEvent) {
		if (swallow) {
			swallow = false;
			e.preventDefault();
			return;
		}
		oncontrol();
	}
	function contextmenu(e: MouseEvent) {
		e.preventDefault();
		cancel();
		menu?.show();
	}
</script>

<div class="room" class:current class:dragging class:target role="group" aria-label={name} data-room={id}>
	<button
		class="main"
		aria-label={`Control ${name}${mode ? `: ${mode.toLowerCase()}` : ''}${title ? `, ${title}${artist ? ` by ${artist}` : ''}` : ''}`}
		aria-current={current ? 'true' : undefined}
		onclick={click}
		onpointerdown={down}
		onpointermove={move}
		onpointerup={cancel}
		onpointercancel={cancel}
		oncontextmenu={contextmenu}
		data-room-control
	>
		<span class="thumb"><Art path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 120)} alt="" label={title || name} /></span>
		<span class="text">
			<span class="name">{name}{#if current}<span class="muted in-use"> · in use</span>{/if}</span>
			<span class="what ellipsis">{title || (on ? 'Nothing playing' : 'Off')}</span>
			{#if artist}<span class="sub ellipsis muted">{artist}</span>{/if}
			<span class="sub muted ellipsis" data-room-meta>
				{[mode, groupNote, sleep !== null ? `sleep in ${formatLeft(sleep)}` : '', alarm ? `alarm ${alarm}` : ''].filter(Boolean).join(' · ')}
			</span>
		</span>
	</button>
	<div class="controls">
		<button class="icon-btn" aria-label={playing ? `Pause ${name}` : `Play ${name}`} onclick={onplaypause} disabled={!status} data-room-play>
			<Icon name={playing ? 'pause' : 'play'} size={22} />
		</button>
		{#if buttonsOnly}
			<span class="vol muted small">Volume on the device</span>
		{:else}
			<input
				class="vol"
				type="range"
				min="0"
				max="100"
				value={volume}
				aria-label={`${name} volume`}
				oninput={(e) => slide(Number(e.currentTarget.value))}
				disabled={!status || !on}
				data-room-volume
			/>
		{/if}
		<button class="icon-btn" class:on aria-label={`${name} power`} aria-pressed={on} onclick={onpower} disabled={!status} data-room-power>
			<Icon name="power" size={20} />
		</button>
		<button
			class="icon-btn grip"
			aria-label={`Drag ${name} onto another room to sync`}
			aria-describedby="rooms-drag-help"
			onpointerdown={ongrip}
			onkeydown={ongripkey}
			data-room-grip
		>
			<Icon name="grip" size={20} />
		</button>
		<RowMenu bind:this={menu} label={`${name} options`} {actions} />
	</div>
</div>

<style>
	.room {
		display: grid;
		gap: 0.35rem;
		padding: 0.6rem;
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-raised);
		transition: box-shadow 0.15s, border-color 0.15s;
	}
	.room.current {
		border-color: var(--accent);
	}
	.room.dragging {
		opacity: 0.7;
		box-shadow: var(--shadow);
	}
	.room.target {
		border-color: var(--accent);
		box-shadow: 0 0 0 3px var(--accent-soft);
	}
	.main {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 64px;
		padding: 0.25rem;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: inherit;
		text-align: left;
		font: inherit;
		-webkit-touch-callout: none;
		user-select: none;
	}
	.main:hover {
		background: var(--bg-sunken);
	}
	.thumb {
		width: 64px;
		flex: none;
	}
	.text {
		display: grid;
		min-width: 0;
		flex: 1;
	}
	.name {
		font-weight: 650;
	}
	.in-use {
		font-weight: 400;
		font-size: 0.85rem;
	}
	.what {
		font-size: 0.95rem;
	}
	.sub {
		font-size: 0.82rem;
	}
	.ellipsis {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.controls {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}
	.vol {
		flex: 1;
		min-width: 0;
		min-height: 44px;
	}
	.icon-btn {
		flex: none;
		width: 44px;
		height: 44px;
	}
	.icon-btn.on {
		color: var(--accent);
	}
	.grip {
		cursor: grab;
		touch-action: none;
	}
	@media (prefers-reduced-motion: reduce) {
		.room {
			transition: none;
		}
	}
</style>
