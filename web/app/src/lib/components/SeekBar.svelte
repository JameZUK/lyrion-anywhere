<script lang="ts">
	// The Now playing seek bar. With a track analysis from the gateway it draws the track's
	// waveform: the played part in the accent colour, the rest muted. Without one (pending,
	// refused, or a stream) it is the plain slider. Either way the control itself is a native
	// range input, so keyboard, screen readers and touch work as everywhere else; the waveform
	// is drawn underneath it.
	import { analyses } from '$lib/state/app.svelte';
	import type { AnalysisClient, AnalysisState } from '$lib/analysis/client';
	import { barCount, bars, barsPath, fraction } from '$lib/analysis/waveform';
	import { duration } from '$lib/lms/format';
	import { haptics } from '$lib/haptics/haptics.svelte';

	interface Props {
		/** The LMS track id (no waveform without one). */
		trackId?: number | string | null;
		/** Shown position, s. */
		value: number;
		/** Track length, s (0 = unknown). */
		total: number;
		disabled?: boolean;
		oninput?: (v: number) => void;
		onchange?: (v: number) => void;
		/** For tests: where analyses come from. */
		client?: AnalysisClient;
	}
	let { trackId = null, value, total, disabled = false, oninput, onchange, client = analyses }: Props = $props();

	const HEIGHT = 40;
	let width = $state(0);
	let analysis = $state<AnalysisState | null>(null);
	const uid = $props.id();

	$effect(() => {
		const id = trackId;
		const known = client.peek(id);
		analysis = known ?? null;
		if (known) return;
		let live = true;
		void client.load(id).then((s) => {
			if (live) analysis = s;
		});
		return () => {
			live = false;
		};
	});

	const ready = $derived(analysis?.status === 'ready' ? analysis.analysis : null);
	const levels = $derived(ready && width > 0 ? bars(ready.peak, barCount(width)) : []);
	const path = $derived(barsPath(levels, Math.max(width, 1), HEIGHT, 1, 2));
	const played = $derived(fraction(value, total) * Math.max(width, 1));
	/** Where the last scrub step was, for a tick at every tenth of the track (haptics). */
	let scrubFrom: number | null = null;
	function scrubbing(v: number) {
		haptics.scrub(scrubFrom ?? value, v, total);
		scrubFrom = v;
		oninput?.(v);
	}

	const valueText = $derived(total > 0 ? `${duration(value)} of ${duration(total)}` : duration(value));
</script>

<div class="seek" class:wave={!!ready && width > 0} bind:clientWidth={width}>
	{#if ready && width > 0}
		<svg class="art" aria-hidden="true" width={width} height={HEIGHT} viewBox={`0 0 ${width} ${HEIGHT}`}>
			<defs>
				<clipPath id={`${uid}-played`}><rect x="0" y="0" width={played} height={HEIGHT} /></clipPath>
			</defs>
			<path class="rest" d={path} />
			<path class="played" d={path} clip-path={`url(#${uid}-played)`} />
			<rect class="head" x={Math.min(Math.max(played - 1, 0), Math.max(width - 2, 0))} y="0" width="2" height={HEIGHT} />
		</svg>
	{/if}
	<input
		type="range"
		min="0"
		max={Math.max(total, 1)}
		step="1"
		{value}
		{disabled}
		aria-label="Position"
		aria-valuetext={valueText}
		oninput={(e) => scrubbing(Number(e.currentTarget.value))}
		onchange={(e) => {
			scrubFrom = null;
			onchange?.(Number(e.currentTarget.value));
		}}
	/>
</div>

<style>
	.seek {
		position: relative;
		width: 100%;
	}
	.seek input {
		width: 100%;
	}
	.wave {
		height: 40px;
		border-radius: 6px;
	}
	.wave .art {
		position: absolute;
		inset: 0;
		display: block;
	}
	/* Over the waveform, the slider is invisible but still the control: it takes every
	   pointer, touch and key. */
	.wave input {
		position: absolute;
		inset: 0;
		height: 100%;
		margin: 0;
		opacity: 0;
		cursor: pointer;
	}
	.wave input:disabled {
		cursor: default;
	}
	.wave:focus-within {
		outline: 2px solid var(--accent);
		outline-offset: 3px;
	}
	.rest {
		fill: var(--fg-muted);
		opacity: 0.45;
	}
	.played {
		fill: var(--accent);
	}
	.head {
		fill: var(--fg);
	}
	@media (forced-colors: active) {
		.rest {
			fill: GrayText;
			opacity: 1;
		}
		.played,
		.head {
			fill: Highlight;
		}
	}
</style>
