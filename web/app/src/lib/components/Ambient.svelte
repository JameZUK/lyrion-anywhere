<script lang="ts">
	// Now playing's ambient background (docs/plans: Wave 1 §2): a small copy of the cover,
	// blurred, drifting slowly behind a scrim of the page colour. The scrim's strength and
	// the art's filter come from the theme (theme/ambient.ts), so every text colour keeps
	// WCAG AA over any cover.
	// It holds still under "reduce motion", on low-power devices, and while the page is
	// hidden; a new cover cross-fades in.
	import { api } from '$lib/state/app.svelte';
	import { ambientStill } from '$lib/theme/motion';

	let { path }: { path: string | null } = $props();

	type Layer = { key: number; src: string };
	let layers = $state<Layer[]>([]);
	let n = 0;
	let reduce = $state(false);
	let hidden = $state(false);
	const still = $derived(reduce || ambientStill(globalThis.navigator));

	$effect(() => {
		const p = path;
		if (!p) {
			layers = [];
			return;
		}
		let live = true;
		const release = api.retainArt?.(p);
		api.art(p).then(
			(src) => {
				if (!live || layers.at(-1)?.src === src) return;
				layers = [...layers.slice(-1), { key: ++n, src }];
			},
			() => {},
		);
		return () => {
			live = false;
			release?.();
		};
	});

	$effect(() => {
		const mq = typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : null;
		const onMotion = () => (reduce = !!mq?.matches);
		const onVisible = () => (hidden = document.visibilityState === 'hidden');
		onMotion();
		onVisible();
		mq?.addEventListener?.('change', onMotion);
		document.addEventListener('visibilitychange', onVisible);
		return () => {
			mq?.removeEventListener?.('change', onMotion);
			document.removeEventListener('visibilitychange', onVisible);
		};
	});
</script>

<div class="ambient" class:still class:paused={hidden} aria-hidden="true" data-ambient>
	<div class="drift">
		{#each layers as l (l.key)}<img class="layer" src={l.src} alt="" draggable="false" />{/each}
	</div>
	<div class="scrim"></div>
</div>

<style>
	/* Fills Now playing's content area (the layout makes it a stacking context of its own),
	   behind everything on it; the sidebar and the bars keep the theme. */
	.ambient {
		position: absolute;
		inset: 0;
		z-index: -1;
		overflow: hidden;
		pointer-events: none;
		background: var(--bg);
	}
	.drift {
		position: absolute;
		inset: -20%;
		will-change: transform;
		animation: drift 70s ease-in-out infinite alternate;
	}
	.layer {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		object-fit: cover;
		/* The theme bounds the art's range (theme/ambient.ts) so the scrim can stay light. */
		filter: blur(48px) var(--ambient-filter, saturate(1.5) brightness(0.5));
		animation: in 1.2s ease-out;
	}
	.scrim {
		position: absolute;
		inset: 0;
		background: color-mix(in srgb, var(--bg) var(--ambient-scrim, 80%), transparent);
	}
	.still .drift {
		animation: none;
		transform: scale(1.1);
	}
	.still .layer {
		animation: none;
	}
	/* High contrast: plain page colour, nothing behind the text. */
	:global(:root[data-theme='contrast']) .ambient .drift {
		display: none;
	}
	.paused .drift {
		animation-play-state: paused;
	}
	@keyframes drift {
		0% {
			transform: translate3d(-4%, -3%, 0) scale(1.1) rotate(0deg);
		}
		50% {
			transform: translate3d(3%, 2%, 0) scale(1.22) rotate(6deg);
		}
		100% {
			transform: translate3d(-2%, 4%, 0) scale(1.15) rotate(-4deg);
		}
	}
	@keyframes in {
		from {
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.drift,
		.layer {
			animation: none !important;
		}
	}
</style>
