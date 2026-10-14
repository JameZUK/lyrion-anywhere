<script lang="ts">
	// Settings › Now playing (Wave 3.5): the lyric peek, the visualiser and its style, ratings
	// (stars or a heart, and clearing them) and haptics. Kept in this browser, except ratings,
	// which are your account's.
	import { lyrics } from '$lib/state/lyrics';
	import { nowPlaying } from '$lib/nowplaying/store.svelte';
	import { STYLES, STYLE_LABELS, type VisualStyle } from '$lib/visual/draw';
	import { haptics } from '$lib/haptics/haptics.svelte';
	import PickerRow from '$lib/components/PickerRow.svelte';
	import RatingsSettings from '$lib/components/RatingsSettings.svelte';
	import Switch from '$lib/components/Switch.svelte';
</script>

<p class="muted small">Kept in this browser (ratings are kept for your account).</p>
<div class="rows">
	<Switch checked={lyrics.peek} onchange={(v) => lyrics.setPeek(v)} label="Show the current lyric line" note="When a track has synced lyrics." data-lyrics-peek-setting />
	<Switch
		checked={nowPlaying.visualiser}
		onchange={(v) => nowPlaying.setVisualiser(v)}
		label="Visualiser"
		note="In place of the cover, while this browser is the player. Off under reduced motion."
		data-visualiser-setting
	/>
	<PickerRow label="Visualiser style" value={nowPlaying.style} options={STYLES.map((s) => ({ value: s, label: STYLE_LABELS[s] }))} onchange={(v) => nowPlaying.setStyle(v as VisualStyle)} id="visualiser-style" />
	<Switch
		checked={haptics.enabled}
		disabled={!haptics.supported}
		onchange={(v) => haptics.set(v)}
		label="Haptics"
		note={haptics.supported ? 'Light ticks when you skip, scrub, change the volume, reorder or rate.' : 'This browser can’t vibrate, so there is nothing to switch on here.'}
		data-haptics-setting
	/>
</div>
<h2 class="sub-h">Ratings</h2>
<RatingsSettings />

<style>
	.rows {
		display: grid;
		gap: 2px;
	}
	.sub-h {
		margin: 1.5rem 0 0.25rem;
		font-size: 1rem;
	}
	.small {
		font-size: 0.85rem;
	}
</style>
