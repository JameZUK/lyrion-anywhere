<script lang="ts">
	// Cover art, loaded through the API (bearer token, never in the URL) when it scrolls
	// into view. Falls back to a placeholder with the item's initial.
	import { api } from '$lib/state/app.svelte';

	let {
		path,
		alt = '',
		label = '',
		rounded = 'var(--radius-sm)',
		eager = false,
	}: { path: string | null; alt?: string; label?: string; rounded?: string; eager?: boolean } = $props();

	let el: HTMLDivElement;
	let src = $state('');
	let failed = $state(false);
	let visible = $state(false);

	$effect(() => {
		if (eager) {
			visible = true;
			return;
		}
		const io = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) {
					visible = true;
					io.disconnect();
				}
			},
			{ rootMargin: '300px' },
		);
		io.observe(el);
		return () => io.disconnect();
	});

	$effect(() => {
		const p = path;
		src = '';
		failed = false;
		if (!p || !visible) return;
		let live = true;
		// Keep this art's blob: URL from being revoked while it is on screen.
		const release = api.retainArt?.(p);
		api.art(p).then(
			(u) => live && (src = u),
			() => live && (failed = true),
		);
		return () => {
			live = false;
			release?.();
		};
	});

	const initial = $derived((label || alt).trim().charAt(0).toUpperCase() || '♪');
</script>

<div class="art" bind:this={el} style:border-radius={rounded}>
	{#if src}
		<img {src} {alt} draggable="false" />
	{:else}
		<span class="ph" aria-hidden="true" class:failed>{initial}</span>
	{/if}
</div>

<style>
	.art {
		position: relative;
		aspect-ratio: 1;
		width: 100%;
		overflow: hidden;
		background: linear-gradient(135deg, var(--bg-sunken), var(--line));
	}
	img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: cover;
		animation: fade 0.25s ease-out;
	}
	.ph {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		font-size: clamp(1rem, 30cqw, 3rem);
		font-weight: 700;
		color: var(--fg-muted);
		opacity: 0.6;
	}
	@keyframes fade {
		from {
			opacity: 0;
		}
	}
</style>
