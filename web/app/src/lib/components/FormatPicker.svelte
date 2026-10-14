<script lang="ts">
	// "Download as…" (Wave 3.5): a one-off format for this download, from the page's ⋯ menu.
	// The usual format comes from Settings › Downloads (one for Wi-Fi, one for mobile data).
	import { defaultFormat, networkType } from '$lib/offline/settings';
	import { FORMATS, FORMAT_LABELS, type Format } from '$lib/offline/types';
	import Sheet from './Sheet.svelte';

	let { onchoose, onclose }: { onchoose: (f: Format | 'auto') => void; onclose: () => void } = $props();
	const auto = defaultFormat();
	const onWifi = networkType() === 'wifi';
	const NOTES: Record<Format, string> = {
		original: 'The file as it is in your library (largest)',
		mp3: 'Plays everywhere',
		opus: 'About 1 MB a minute',
	};

	function choose(f: Format | 'auto') {
		onclose();
		onchoose(f);
	}
</script>

<Sheet title="Download as" {onclose}>
	<div class="rows" data-format-picker>
		<button class="row" onclick={() => choose('auto')} data-format="auto">
			<span>{FORMAT_LABELS[auto]}<small>Your usual format on {onWifi ? 'Wi-Fi' : 'mobile data'} (Settings › Downloads)</small></span>
		</button>
		{#each FORMATS as f (f)}
			<button class="row" onclick={() => choose(f)} data-format={f}><span>{FORMAT_LABELS[f]}<small>{NOTES[f]}</small></span></button>
		{/each}
	</div>
</Sheet>

<style>
	.rows {
		display: grid;
		gap: 2px;
	}
	.row {
		display: flex;
		align-items: center;
		width: 100%;
		min-height: 52px;
		padding: 8px 10px;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: var(--fg);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	small {
		display: block;
		color: var(--fg-muted);
		font-size: 0.82rem;
	}
</style>
