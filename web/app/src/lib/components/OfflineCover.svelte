<script lang="ts">
	// An album cover kept with the downloads: read from this device's storage, so it shows
	// with no connection. Its blob: URL is given back when the cover goes.
	import { offlineStore } from '#lib/offline/index.svelte';

	let { user, albumId, label = '' }: { user: string; albumId: number | null; label?: string } = $props();

	let src = $state('');

	$effect(() => {
		const u = user;
		const id = albumId;
		src = '';
		if (!u || id === null) return;
		let live = true;
		let url = '';
		offlineStore.getCover(u, id).then(
			(blob) => {
				if (!live || !blob) return;
				url = URL.createObjectURL(blob);
				src = url;
			},
			() => {},
		);
		return () => {
			live = false;
			if (url) URL.revokeObjectURL(url);
		};
	});

	const initial = $derived(label.trim().charAt(0).toUpperCase() || '♪');
</script>

<div class="art">
	{#if src}
		<img {src} alt="" draggable="false" />
	{:else}
		<span class="ph" aria-hidden="true">{initial}</span>
	{/if}
</div>

<style>
	.art {
		position: relative;
		aspect-ratio: 1;
		width: 100%;
		overflow: hidden;
		border-radius: var(--radius-sm);
		background: linear-gradient(135deg, var(--bg-sunken), var(--line));
	}
	img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: cover;
	}
	.ph {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		font-size: 1.4rem;
		font-weight: 700;
		color: var(--fg-muted);
		opacity: 0.6;
	}
</style>
