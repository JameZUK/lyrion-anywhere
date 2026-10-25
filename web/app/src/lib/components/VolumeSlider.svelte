<script lang="ts">
	// The volume of the player in use, in Play on… on phones (Wave 3.5: phone Now playing has no
	// volume row; the hardware keys and this slider set it). It sets that player's volume through
	// LMS: for another player, its own; when this browser is the player, this browser's (which
	// follows LMS's volume). When this device's own buttons set the level (iPhone and iPad), it
	// says so instead.
	import { app } from '#lib/state/app.svelte';
	import * as cmd from '#lib/lms/commands';
	import { device } from '#lib/player/device.svelte';
	import { DEVICE_BUTTONS_NOTE, FIXED_LEVEL_NOTE, levelFixed } from '#lib/player/volume';
	import { isThisDevice } from '#lib/player/outputs';
	import { haptics } from '#lib/haptics/haptics.svelte';
	import Icon from './Icon.svelte';

	let { disabled = false }: { disabled?: boolean } = $props();

	let volume = $state<number | null>(null);
	let timer: ReturnType<typeof setTimeout> | undefined;

	const here = $derived(isThisDevice(app.playerId));
	const name = $derived(here ? 'this device' : app.nameOf(app.playerId) || 'this player');
	const label = $derived(`Volume on ${name}`);
	// LMS reports a muted player's level as negative.
	const vol = $derived(Math.min(100, Math.abs(Number(volume ?? app.now?.['mixer volume'] ?? 0)) || 0));
	const buttonsOnly = $derived(device.buttonsOnly(app.playerId));
	// LMS keeps the player's output level at 100% (its Volume Control): no volume to set.
	const fixed = $derived(levelFixed(app.now));

	function set(v: number) {
		const to = Math.max(0, Math.min(100, Math.round(Number.isFinite(v) ? v : 0)));
		haptics.volume(vol, to);
		volume = to;
		clearTimeout(timer);
		timer = setTimeout(async () => {
			await app.send(cmd.volume(to));
			volume = null;
		}, 150);
	}
	// Another player chosen meanwhile: a pending level was for the old one.
	$effect(() => {
		void app.playerId;
		return () => {
			clearTimeout(timer);
			volume = null;
		};
	});
</script>

<div class="vol" data-volume-slider>
	{#if fixed}
		<p class="note muted" data-volume-note data-fixed-level-note><Icon name="volume" size={20} /><span>{FIXED_LEVEL_NOTE}</span></p>
	{:else if buttonsOnly}
		<p class="note muted" data-volume-note><Icon name="volume" size={20} /><span>{DEVICE_BUTTONS_NOTE}</span></p>
	{:else}
		<Icon name={vol > 0 ? 'volume' : 'mute'} size={20} />
		<input type="range" min="0" max="100" step="1" value={vol} aria-label={label} oninput={(e) => set(Number(e.currentTarget.value))} disabled={disabled || !app.now} />
		<span class="value muted" aria-hidden="true">{Math.round(vol)}</span>
	{/if}
</div>

<style>
	.vol {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 48px;
		padding: 0 0.75rem;
		color: var(--fg-muted);
	}
	input {
		flex: 1;
		min-width: 0;
		height: 44px;
		margin: 0;
		accent-color: var(--accent);
	}
	.value {
		width: 2.5ch;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.note {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin: 0;
		font-size: 0.85rem;
	}
</style>
