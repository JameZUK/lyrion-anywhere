<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import { useDetailTitle } from '$lib/title.svelte';
	import { page } from '$app/state';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, isRemote, textOf } from '$lib/lms/format';
	import type { Album, AlbumsResult, TitlesResult, Track } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import TrackList from '$lib/components/TrackList.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import InfoText from '$lib/components/InfoText.svelte';
	import { sanitiseAlbumInfo } from '$lib/info/info';
	import type { TrackBadge } from '$lib/components/track-badge';
	import DownloadBar from '$lib/components/DownloadBar.svelte';
	import { downloads, offlinePlayer } from '$lib/offline/index.svelte';
	import { controls } from '$lib/player/controls.svelte';

	let ctx = $state<{ kind: cmd.InfoKind; id: number; title: string } | null>(null);

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

	// The album's review (Music & Artist Information plugin, through the gateway): loaded
	// lazily; none, or a failure, is simply not shown.
	let review = $state<string | null>(null);
	$effect(() => {
		const id = albumId;
		review = null;
		if (id === null) return;
		let live = true;
		api.info?.('album', id).then(
			(v) => live && (review = sanitiseAlbumInfo(v).review),
			() => {},
		);
		return () => (live = false);
	});

	const artistHref = $derived(cmd.link('artist', album?.artist_id));
	const totalTime = $derived(tracks.reduce((s, t) => s + (Number(t.duration) || 0), 0));
	const many = $derived(new Set(tracks.map((t) => t.artist)).size > 1);

	// Downloads (SPEC §8): library tracks only (never radio or streamed ones).
	const downloadable = $derived(tracks.filter((t) => typeof t.id === 'number' && !isRemote(t.remote)));
	const requests = $derived(
		albumId === null ? [] : downloadable.map((t) => ({ id: t.id, albumId, duration: t.duration, filesize: t.filesize })),
	);
	let trackMsg = $state('');

	/** A single track: download it on its own, or remove it (unless a playlist needs it). */
	function trackDownload(t: Track): string | undefined {
		if (typeof t.id !== 'number' || isRemote(t.remote)) return undefined;
		if (downloads.savedById(t.id)) return 'Remove download';
		const it = downloads.item(t.id);
		if (it && it.status !== 'done' && it.status !== 'failed') return undefined;
		return downloads.online ? 'Download' : undefined;
	}
	async function toggleTrack(t: Track) {
		trackMsg = '';
		const s = downloads.savedById(t.id);
		if (s) {
			const r = await downloads.removeTracks([s.key]);
			offlinePlayer.forget(r.removed);
			if (r.kept) trackMsg = 'This track stays: a downloaded playlist still needs it.';
			return;
		}
		const r = await downloads.request([{ id: t.id, albumId, duration: t.duration, filesize: t.filesize }], { ref: 'track' });
		trackMsg = r.refused ?? '';
	}
	function badge(t: Track): TrackBadge | undefined {
		if (downloads.savedById(t.id)) return { kind: 'done', label: 'Downloaded' };
		const it = typeof t.id === 'number' ? downloads.item(t.id) : undefined;
		if (!it) return undefined;
		if (it.status === 'failed') return { kind: 'failed', label: it.message || 'Download failed' };
		if (it.status === 'paused') return { kind: 'busy', label: 'Download paused' };
		if (downloads.isHeld(it)) return { kind: 'busy', label: 'Waiting for Wi-Fi' };
		if (it.status === 'active' && it.total) return { kind: 'busy', label: `Downloading ${Math.round((100 * it.received) / it.total)}%` };
		return { kind: 'busy', label: 'Waiting to download' };
	}
	useDetailTitle(() => textOf(album?.album));
</script>

<section class="page">
	<BackButton parent="/albums" />
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
					<!-- These need the server (More is LMS's own menu): disabled while it is out of reach. -->
					<button class="btn primary" onclick={() => app.queueUp({ album: id }, 'play')} {...controls.needsServer()}><Icon name="play" size={18} />Play</button>
					<button class="btn" onclick={() => app.queueUp({ album: id }, 'next')} {...controls.needsServer()}><Icon name="playnext" size={18} />Play next</button>
					<button class="btn" onclick={() => app.queueUp({ album: id }, 'add')} {...controls.needsServer()}><Icon name="plus" size={18} />Add to queue</button>
					<button class="btn" onclick={() => (ctx = { kind: 'album', id, title: textOf(album?.album) })} {...controls.needsServer()} data-album-more><Icon name="more" size={18} />More</button>
				</div>
				<DownloadBar tracks={requests} ref={`album:${id}`} onremoved={(keys) => offlinePlayer.forget(keys)} />
				{#if trackMsg}<p class="small muted" role="alert">{trackMsg}</p>{/if}
			</div>
		</header>
		{#if review}
			<div class="review"><InfoText heading="Review" text={review} /></div>
		{/if}
		<TrackList
			{tracks}
			showArtist={many}
			disabled={!app.canQueue}
			onplay={(_t, i) => app.queueUp({ album: id }, 'play', i)}
			onnext={(t) => t.id && app.queueUp({ track: t.id }, 'next')}
			onadd={(t) => t.id && app.queueUp({ track: t.id }, 'add')}
			onmore={(t) => t.id && (ctx = { kind: 'track', id: t.id, title: textOf(t.title) })}
			{badge}
			download={trackDownload}
			ondownload={toggleTrack}
		/>
	{:else}
		<p class="muted">Loading…</p>
	{/if}
</section>

{#if ctx}<ContextMenu kind={ctx.kind} id={ctx.id} title={ctx.title} onclose={() => (ctx = null)} />{/if}

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
	.review {
		margin-top: -0.5rem;
	}
	.small {
		font-size: 0.85rem;
		flex-basis: 100%;
		margin: 0;
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
