<script lang="ts" module>
	import { ColourCache } from '#lib/visual/glow';
	/** Each cover's colours, worked out once per album (shared by every Now playing). */
	const cache = new ColourCache();
</script>

<script lang="ts">
	// The Glow halo (lib/visual/glow.ts): soft lobes in the cover's own colours behind it,
	// turning slowly while music plays. Its size and opacity come from the page's
	// `--glow-radius` and `--glow-opacity`, which the visualiser sets on each beat. It holds
	// still, at rest, under "reduce motion", and goes in the high-contrast theme. Decorative.
	import { api } from '#lib/state/app.svelte';
	import { dominantColours, haloColours, ROTATION_MS, type Rgb } from '#lib/visual/glow';

	let { path, playing = false, still = false }: { path: string | null; playing?: boolean; still?: boolean } = $props();

	/** The analysed copy of the cover is this small. */
	const SAMPLE = 32;
	let colours = $state<Rgb[]>([]);

	/** The cover's pixels, through a small canvas (its art is same-origin, so it can be read). */
	async function analyse(src: string): Promise<Rgb[]> {
		if (typeof Image === 'undefined' || typeof document === 'undefined') return [];
		const img = new Image();
		img.decoding = 'async';
		img.src = src;
		await img.decode();
		const c = document.createElement('canvas');
		c.width = SAMPLE;
		c.height = SAMPLE;
		const ctx = c.getContext('2d', { willReadFrequently: true });
		if (!ctx) return [];
		ctx.drawImage(img, 0, 0, SAMPLE, SAMPLE);
		return dominantColours(ctx.getImageData(0, 0, SAMPLE, SAMPLE).data);
	}

	$effect(() => {
		const p = path;
		if (!p) {
			colours = [];
			return;
		}
		const known = cache.get(p);
		if (known) {
			colours = known;
			return;
		}
		colours = [];
		let live = true;
		const release = api.retainArt?.(p);
		api.art(p)
			.then(analyse)
			.then(
				(found) => {
					cache.set(p, found);
					if (live) colours = found;
				},
				() => {}, // no art, or it can't be read: the accent stays
			)
			.finally(() => release?.());
		return () => {
			live = false;
		};
	});

	const lobes = $derived(haloColours(colours));
</script>

<div class="halo" class:still class:spinning={playing && !still} aria-hidden="true" data-glow-halo data-colours={colours.length} style:--turn={`${ROTATION_MS}ms`}>
	<div class="spin">
		{#each lobes as c, i (i)}<span class="lobe" style:--c={c} style:--a={`${i * 120}deg`}></span>{/each}
	</div>
</div>

<style>
	.halo {
		position: absolute;
		inset: -16%;
		z-index: 0;
		pointer-events: none;
		opacity: var(--glow-opacity, 0.35);
		transform: scale(var(--glow-radius, 1));
		/* Fades out well inside its box, so it never sits hard behind the text below. */
		mask-image: radial-gradient(closest-side, #000 55%, transparent 100%);
		-webkit-mask-image: radial-gradient(closest-side, #000 55%, transparent 100%);
	}
	.spin {
		position: absolute;
		inset: 0;
		animation: turn var(--turn, 24s) linear infinite;
		animation-play-state: paused;
	}
	.spinning .spin {
		animation-play-state: running;
	}
	.lobe {
		position: absolute;
		inset: 12%;
		border-radius: 50%;
		background: radial-gradient(circle at center, var(--c) 0%, color-mix(in srgb, var(--c) 45%, transparent) 38%, transparent 68%);
		transform: rotate(var(--a)) translateY(-11%);
		filter: blur(18px);
	}
	@keyframes turn {
		to {
			transform: rotate(1turn);
		}
	}
	/* Reduced motion: a still halo at rest; no pulse and no turning. */
	.still,
	.still .spin {
		animation: none;
	}
	.still {
		opacity: 0.35;
		transform: none;
	}
	@media (prefers-reduced-motion: reduce) {
		.halo {
			opacity: 0.35;
			transform: none;
		}
		.spin {
			animation: none;
		}
	}
	:global(:root[data-theme='contrast']) .halo {
		display: none;
	}
</style>
