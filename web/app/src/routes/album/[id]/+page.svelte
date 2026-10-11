<script lang="ts">
	import { page } from '$app/state';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, isRemote, textOf } from '$lib/lms/format';
	import type { Album, AlbumsResult, TitlesResult, Track } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import TrackList from '$lib/components/TrackList.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import type { TrackBadge } from '$lib/components/track-badge';
	import { downloads } from '$lib/offline/index.svelte';
	import { defaultFormat, networkType } from '$lib/offline/settings';
	import { FORMATS, FORMAT_LABELS, type Format } from '$lib/offline/types';

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

	const artistHref = $derived(cmd.link('artist', album?.artist_id));
	const totalTime = $derived(tracks.reduce((s, t) => s + (Number(t.duration) || 0), 0));
	const many = $derived(new Set(tracks.map((t) => t.artist)).size > 1);

	// Downloads (SPEC §8): library tracks only (never radio or streamed ones).
	let choice = $state<Format | 'auto'>('auto');
	const auto = defaultFormat();
	const onWifi = networkType() === 'wifi';
	const downloadable = $derived(tracks.filter((t) => typeof t.id === 'number' && !isRemote(t.remote)));
	const savedCount = $derived(downloadable.filter((t) => downloads.savedById(t.id)).length);
	const allSaved = $derived(downloadable.length > 0 && savedCount === downloadable.length);
	const progress = $derived(albumId !== null ? downloads.album(albumId) : null);
	const firstError = $derived(albumId !== null ? downloads.items.find((i) => i.albumId === albumId && i.status === 'failed')?.message : undefined);

	function download(id: number) {
		downloads.enqueue(downloadable.map((t) => ({ id: t.id, albumId: id })), choice);
	}
	async function removeDownload(id: number) {
		const keys = new Set(downloads.savedAlbum(id).map((t) => t.key));
		for (const t of downloadable) {
			const s = downloads.savedById(t.id);
			if (s) keys.add(s.key);
		}
		await downloads.cancelAll(id);
		await downloads.remove([...keys]);
	}
	function badge(t: Track): TrackBadge | undefined {
		if (downloads.savedById(t.id)) return { kind: 'done', label: 'Downloaded' };
		const it = typeof t.id === 'number' ? downloads.item(t.id) : undefined;
		if (!it) return undefined;
		if (it.status === 'failed') return { kind: 'failed', label: it.message || 'Download failed' };
		if (it.status === 'paused') return { kind: 'busy', label: 'Download paused' };
		if (it.status === 'active' && it.total) return { kind: 'busy', label: `Downloading ${Math.round((100 * it.received) / it.total)}%` };
		return { kind: 'busy', label: 'Waiting to download' };
	}
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
					<button class="btn" onclick={() => (ctx = { kind: 'album', id, title: textOf(album?.album) })}><Icon name="more" size={18} />More</button>
				</div>
				{#if downloadable.length}
					<div class="dl" data-testid="album-download">
						{#if progress?.busy}
							<span class="dl-state" role="status">Downloading {progress.done} of {progress.count} · {Math.round(progress.fraction * 100)}%</span>
							<progress max="1" value={progress.fraction} aria-label="Download progress"></progress>
							<button class="btn" onclick={() => downloads.pauseAll(id)}><Icon name="pause" size={18} />Pause</button>
							<button class="btn" onclick={() => downloads.cancelAll(id)}><Icon name="close" size={18} />Cancel</button>
						{:else if progress && progress.paused > 0}
							<span class="dl-state" role="status">Download paused ({progress.done} of {progress.count})</span>
							<button class="btn" onclick={() => downloads.resumeAll(id)}><Icon name="play" size={18} />Resume</button>
							<button class="btn" onclick={() => downloads.cancelAll(id)}><Icon name="close" size={18} />Cancel</button>
						{:else if allSaved}
							<span class="dl-state done" role="status"><Icon name="check" size={18} />Downloaded</span>
							<button class="btn" onclick={() => removeDownload(id)}><Icon name="trash" size={18} />Remove download</button>
						{:else}
							<button class="btn" onclick={() => download(id)} disabled={!downloads.online}>
								<Icon name="download" size={18} />{savedCount ? 'Download the rest' : 'Download'}
							</button>
							<label class="fmt">
								<span class="sr-only">Download format</span>
								<select bind:value={choice}>
									<option value="auto">{FORMAT_LABELS[auto]} ({onWifi ? 'Wi-Fi' : 'mobile data'} default)</option>
									{#each FORMATS as f (f)}<option value={f}>{FORMAT_LABELS[f]}</option>{/each}
								</select>
							</label>
							{#if savedCount}
								<span class="dl-state muted">{savedCount} of {downloadable.length} downloaded</span>
								<button class="btn" onclick={() => removeDownload(id)}><Icon name="trash" size={18} />Remove download</button>
							{/if}
						{/if}
						{#if progress?.failed && !progress.busy}
							<p class="error small" role="alert">
								{progress.failed} {progress.failed === 1 ? 'track' : 'tracks'} couldn’t be downloaded: {firstError}
								<button class="btn" onclick={() => downloads.resumeAll(id)}>Try again</button>
							</p>
						{/if}
					</div>
				{/if}
			</div>
		</header>
		<TrackList
			{tracks}
			showArtist={many}
			onplay={(_t, i) => app.queueUp({ album: id }, 'play', i)}
			onnext={(t) => t.id && app.queueUp({ track: t.id }, 'next')}
			onadd={(t) => t.id && app.queueUp({ track: t.id }, 'add')}
			onmore={(t) => t.id && (ctx = { kind: 'track', id: t.id, title: textOf(t.title) })}
			{badge}
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
	.dl {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
		margin-top: 0.75rem;
	}
	.dl-state {
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		font-weight: 550;
	}
	.dl-state.done {
		color: var(--accent);
	}
	.dl progress {
		width: 8rem;
		accent-color: var(--accent);
	}
	.fmt select {
		padding: 0.5em 0.6em;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		color: var(--fg);
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
