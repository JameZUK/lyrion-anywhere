<script lang="ts">
	import { page } from '$app/state';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, textOf } from '$lib/lms/format';
	import type { Album, AlbumsResult, TitlesResult, Track } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import TrackList from '$lib/components/TrackList.svelte';

	let album = $state<Album | null>(null);
	let tracks = $state<Track[]>([]);
	let error = $state('');

	const albumId = $derived.by(() => {
		try {
			return cmd.id(page.params.id);
		} catch {
			return null;
		}
	});

	$effect(() => {
		const id = albumId;
		album = null;
		tracks = [];
		error = '';
		if (id === null) {
			error = 'Not an album.';
			return;
		}
		let live = true;
		Promise.all([api.rpc<AlbumsResult>('', cmd.albumById(id)), api.rpc<TitlesResult>('', cmd.albumTracks(id))]).then(
			([a, t]) => {
				if (!live) return;
				album = a.albums_loop?.[0] ?? null;
				tracks = t.titles_loop ?? [];
				if (!album) error = 'Album not found.';
			},
			(e) => live && (error = (e as Error).message),
		);
		return () => (live = false);
	});

	const artistHref = $derived(cmd.link('artist', album?.artist_id));
	const totalTime = $derived(tracks.reduce((s, t) => s + (Number(t.duration) || 0), 0));
	const many = $derived(new Set(tracks.map((t) => t.artist)).size > 1);
</script>

<section class="page">
	{#if error}
		<p class="error">{error}</p>
	{:else if album && albumId !== null}
		{@const id = albumId}
		<header>
			<div class="cover"><Art path={cmd.coverPath(album.artwork_track_id, 600)} alt="" label={textOf(album.album)} rounded="var(--radius)" eager /></div>
			<div class="meta">
				<p class="kind muted">Album</p>
				<h1>{textOf(album.album)}</h1>
				<p class="by">
					{#if artistHref}<a href={artistHref}>{textOf(album.artist)}</a>{:else}{textOf(album.artist)}{/if}
				</p>
				<p class="muted">{[textOf(album.year), `${tracks.length} tracks`, duration(totalTime)].filter(Boolean).join(' · ')}</p>
				<div class="actions">
					<button class="btn primary" onclick={() => app.queueUp({ album: id }, 'play')}><Icon name="play" size={18} />Play</button>
					<button class="btn" onclick={() => app.queueUp({ album: id }, 'next')}><Icon name="playnext" size={18} />Play next</button>
					<button class="btn" onclick={() => app.queueUp({ album: id }, 'add')}><Icon name="plus" size={18} />Add to queue</button>
				</div>
			</div>
		</header>
		<TrackList
			{tracks}
			showArtist={many}
			onplay={(_t, i) => app.queueUp({ album: id }, 'play', i)}
			onnext={(t) => t.id && app.queueUp({ track: t.id }, 'next')}
			onadd={(t) => t.id && app.queueUp({ track: t.id }, 'add')}
		/>
	{:else}
		<p class="muted">Loading…</p>
	{/if}
</section>

<style>
	header {
		display: grid;
		grid-template-columns: minmax(140px, 260px) minmax(0, 1fr);
		gap: 1.75rem;
		align-items: end;
		margin-bottom: 1.75rem;
	}
	.cover {
		box-shadow: var(--shadow);
		border-radius: var(--radius);
	}
	.meta p {
		margin: 0.2rem 0;
	}
	.kind {
		text-transform: uppercase;
		letter-spacing: 0.08em;
		font-size: 0.75rem;
		font-weight: 650;
	}
	h1 {
		font-size: clamp(1.6rem, 3vw, 2.6rem);
		margin: 0.2rem 0;
	}
	.by {
		font-weight: 600;
		font-size: 1.05rem;
	}
	.by a:hover {
		text-decoration: underline;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		margin-top: 1rem;
	}
	@media (max-width: 600px) {
		header {
			grid-template-columns: minmax(0, 1fr);
		}
		.cover {
			width: min(70vw, 280px);
			justify-self: center;
		}
	}
</style>
