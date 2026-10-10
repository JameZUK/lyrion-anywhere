<script lang="ts">
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { Artist, ArtistsResult } from '$lib/lms/types';

	const PAGE = 200;
	let artists = $state<Artist[]>([]);
	let total = $state<number | null>(null);
	let loading = false;
	let error = $state('');
	let sentinel = $state<HTMLDivElement>();

	async function more() {
		if (loading || (total !== null && artists.length >= total)) return;
		loading = true;
		try {
			const r = await api.rpc<ArtistsResult>('', cmd.artists(artists.length, PAGE));
			artists = [...artists, ...(r.artists_loop ?? [])];
			total = r.count ?? artists.length;
		} catch (e) {
			error = (e as Error).message;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void app.libraryVersion;
		untrack(() => {
			artists = [];
			total = null;
			void more();
		});
	});

	$effect(() => {
		if (!sentinel) return;
		const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && void more(), { rootMargin: '800px' });
		io.observe(sentinel);
		return () => io.disconnect();
	});
</script>

<section class="page">
	<h1>Artists</h1>
	{#if total !== null}<p class="muted">{total.toLocaleString('en-GB')} artists</p>{/if}
	{#if error}<p class="error">{error}</p>{/if}
	<ul class="list">
		{#each artists as a (a.id)}
			<li><a href={cmd.link('artist', a.id) ?? '#/artists'}><span class="avatar" aria-hidden="true">{textOf(a.artist).charAt(0).toUpperCase()}</span><span class="ellipsis">{textOf(a.artist)}</span></a></li>
		{/each}
	</ul>
	{#if total === null || artists.length < total}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
</section>

<style>
	.list {
		list-style: none;
		padding: 0;
		margin: 1rem 0 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
		gap: 0.25rem 1rem;
	}
	a {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.45rem 0.5rem;
		border-radius: var(--radius-sm);
		min-width: 0;
	}
	a:hover {
		background: var(--bg-sunken);
	}
	.avatar {
		flex: none;
		width: 36px;
		height: 36px;
		border-radius: 50%;
		display: grid;
		place-items: center;
		background: var(--accent-soft);
		color: var(--accent);
		font-weight: 700;
	}
</style>
