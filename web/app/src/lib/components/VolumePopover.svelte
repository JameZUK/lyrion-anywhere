<script lang="ts">
	// Volume on Now playing for phones: a button in the bottom row that opens a small popover
	// with a vertical slider. It sets the volume of the player in use through LMS: for another
	// player, that player's volume; when this browser is the player, this browser's own (which
	// follows LMS's volume, as the slider always has). When this device's own buttons set the
	// level (iPhone and iPad), the popover says so instead. Escape or a tap outside closes it,
	// and focus goes back to the button.
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { device } from '$lib/player/device.svelte';
	import { DEVICE_BUTTONS_NOTE } from '$lib/player/volume';
	import { isThisDevice } from '$lib/player/outputs';
	import { haptics } from '$lib/haptics/haptics.svelte';
	import Icon from './Icon.svelte';

	let { disabled = false }: { disabled?: boolean } = $props();

	const uid = $props.id();
	let open = $state(false);
	let btn = $state<HTMLButtonElement>();
	let pop = $state<HTMLDivElement>();
	let volume = $state<number | null>(null);
	let timer: ReturnType<typeof setTimeout> | undefined;

	const here = $derived(isThisDevice(app.playerId));
	const name = $derived(here ? 'this device' : app.nameOf(app.playerId) || 'this player');
	const label = $derived(`Volume on ${name}`);
	const vol = $derived(Math.abs(Number(volume ?? app.now?.['mixer volume'] ?? 0)) || 0);
	const buttonsOnly = $derived(device.buttonsOnly(app.playerId));

	function set(v: number) {
		haptics.volume(vol, v);
		volume = v;
		clearTimeout(timer);
		timer = setTimeout(async () => {
			await app.send(cmd.volume(v));
			volume = null;
		}, 150);
	}

	function show() {
		open = true;
		queueMicrotask(() => (pop?.querySelector<HTMLElement>('input') ?? pop)?.focus());
	}
	function close(refocus = true) {
		if (!open) return;
		open = false;
		if (refocus) btn?.focus();
	}
	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			close();
		}
	}
	// A tap anywhere else closes it (the button toggles it itself); so does focus leaving it.
	$effect(() => {
		if (!open) return;
		const outside = (e: PointerEvent) => {
			const t = e.target as Node;
			if (!pop?.contains(t) && !btn?.contains(t)) close(false);
		};
		document.addEventListener('pointerdown', outside, true);
		return () => document.removeEventListener('pointerdown', outside, true);
	});
	function onfocusout(e: FocusEvent) {
		const to = e.relatedTarget as Node | null;
		if (to && !pop?.contains(to) && !btn?.contains(to)) close(false);
	}
	// Another player chosen meanwhile: this popover was for the old one.
	$effect(() => {
		void app.playerId;
		return () => (open = false);
	});
	$effect(() => () => clearTimeout(timer));
</script>

<span class="volume">
	<button
		bind:this={btn}
		class="np-act"
		aria-haspopup="dialog"
		aria-expanded={open}
		aria-controls={open ? `${uid}-pop` : undefined}
		aria-label={label}
		onclick={() => (open ? close() : show())}
		{disabled}
		data-volume-button
	>
		<Icon name={vol > 0 || buttonsOnly ? 'volume' : 'mute'} size={24} /><span class="np-act-label" aria-hidden="true">Volume</span>
	</button>
	{#if open}
		<div class="pop" role="dialog" id={`${uid}-pop`} aria-label={label} tabindex="-1" bind:this={pop} {onkeydown} {onfocusout} data-volume-popover>
			{#if buttonsOnly}
				<p class="note" data-volume-note>{DEVICE_BUTTONS_NOTE}</p>
			{:else}
				<span class="value" aria-hidden="true">{Math.round(vol)}</span>
				<input type="range" min="0" max="100" step="1" value={vol} aria-label={label} oninput={(e) => set(Number(e.currentTarget.value))} disabled={!app.now} />
				<span class="who">{here ? 'This device' : name}</span>
			{/if}
		</div>
	{/if}
</span>

<style>
	.volume {
		position: relative;
		display: grid;
	}
	.pop {
		position: absolute;
		right: 0;
		bottom: calc(100% + 8px);
		z-index: 25;
		width: 88px;
		padding: 0.8rem 0.25rem;
		display: grid;
		justify-items: center;
		gap: 0.6rem;
		border-radius: 18px;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		box-shadow: var(--shadow);
	}
	.pop:focus-visible {
		outline: 2px solid var(--accent);
	}
	.value {
		font-family: var(--mono);
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	/* A vertical slider: up is louder. 44 px wide, so it is an easy target. */
	input {
		writing-mode: vertical-lr;
		direction: rtl;
		width: 44px;
		height: min(170px, 30vh);
		margin: 0;
		accent-color: var(--accent);
	}
	.who,
	.note {
		margin: 0;
		font-size: 0.72rem;
		color: var(--fg-muted);
		text-align: center;
		overflow-wrap: anywhere;
	}
	.note {
		font-size: 0.8rem;
	}
</style>
