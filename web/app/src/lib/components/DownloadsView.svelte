<script lang="ts">
	// What this device has downloaded for the signed-in user (or, offline, the last one):
	// albums with covers and sizes, storage use, downloads in progress, and playback.
	import { downloads, offlinePlayer } from '$lib/offline/index.svelte';
	import { formatBytes, storageInfo, type StorageInfo } from '$lib/offline/settings';
	import { FORMAT_LABELS, type StoredTrack } from '$lib/offline/types';
	import { duration } from '$lib/lms/format';
	import { playable } from '$lib/offline/player.svelte';
	import OfflineCover from './OfflineCover.svelte';
	import Icon from './Icon.svelte';

	let { offline = false }: { offline?: boolean } = $props();

	interface Group {
		id: string;
		albumId: number | null;
		album: string;
		artist: string;
		tracks: StoredTrack[];
		size: number;
	}

	const user = $derived(downloads.user ?? '');
	const groups = $derived.by(() => {
		const map = new Map<string, Group>();
		for (const t of downloads.saved) {
			const id = t.albumId !== null ? `a${t.albumId}` : `n${t.album}`;
			let g = map.get(id);
			if (!g) map.set(id, (g = { id, albumId: t.albumId, album: t.album || 'Unknown album', artist: '', tracks: [], size: 0 }));
			g.tracks.push(t);
			g.size += t.size;
		}
		for (const g of map.values()) {
			g.tracks.sort((a, b) => (a.tracknum ?? 1e9) - (b.tracknum ?? 1e9) || a.title.localeCompare(b.title));
			const artists = new Set(g.tracks.map((t) => t.artist));
			g.artist = artists.size === 1 ? (g.tracks[0]?.artist ?? '') : 'Various artists';
		}
		return [...map.values()].sort((a, b) => a.album.localeCompare(b.album));
	});
	const pending = $derived(downloads.items.filter((i) => i.status !== 'done'));

	let info = $state<StorageInfo>({});
	$effect(() => {
		void downloads.saved.length; // refresh after changes
		let live = true;
		storageInfo().then((i) => live && (info = i));
		return () => (live = false);
	});

	let open = $state<string | null>(null);
	let confirmAll = $state(false);
	let probe: HTMLAudioElement | null = null;
	const canPlay = (t: StoredTrack) => {
		probe ??= typeof document !== 'undefined' ? document.createElement('audio') : null;
		return !probe || typeof probe.canPlayType !== 'function' || playable(probe, t);
	};

	const ios = typeof navigator !== 'undefined' && /iPhone|iPad|iPod/.test(navigator.userAgent);
	const standalone = typeof matchMedia === 'function' && matchMedia('(display-mode: standalone)').matches;

	async function remove(keys: string[]) {
		offlinePlayer.forget(keys);
		await downloads.remove(keys);
	}
	async function removeAll() {
		confirmAll = false;
		offlinePlayer.stop();
		await downloads.removeAll();
	}
	const pct = (n: number) => `${Math.round(n * 100)}%`;
</script>

<section class="page downloads">
	<h1>Downloads</h1>
	{#if offline}
		<p class="note" role="status"><Icon name="offline" size={18} /> You’re offline. Your downloads still play.</p>
	{/if}
	{#if downloads.error}<p class="error" role="alert">{downloads.error}</p>{/if}

	<div class="card storage">
		<p>
			<strong>{downloads.saved.length} {downloads.saved.length === 1 ? 'track' : 'tracks'}, {formatBytes(downloads.usedBytes)}</strong>
			{#if info.quota}<span class="muted"> · this app uses {formatBytes(info.usage ?? 0)} of {formatBytes(info.quota)} available</span>{/if}
		</p>
		{#if info.quota}
			<div class="meter" role="meter" aria-label="Storage used" aria-valuemin="0" aria-valuemax={info.quota} aria-valuenow={info.usage ?? 0}>
				<span style:width={pct(Math.min(1, (info.usage ?? 0) / info.quota))}></span>
			</div>
		{/if}
		<p class="muted small">
			Downloads stay on this device until you remove them, and are kept apart for each account.
			{#if info.persisted === false}The browser may clear them if the device runs short of space.{/if}
			{#if ios && !standalone}On iPhone and iPad they are only kept reliably when the app is added to the Home Screen.{/if}
		</p>
		{#if downloads.saved.length}
			{#if confirmAll}
				<div class="row" role="group" aria-label="Confirm removing all downloads">
					<span>Remove all {downloads.saved.length} downloaded tracks from this device?</span>
					<button class="btn danger" onclick={removeAll}><Icon name="trash" size={18} />Remove all</button>
					<button class="btn" onclick={() => (confirmAll = false)}>Cancel</button>
				</div>
			{:else}
				<button class="btn" onclick={() => (confirmAll = true)}><Icon name="trash" size={18} />Remove all…</button>
			{/if}
		{/if}
	</div>

	{#if pending.length}
		<div class="card">
			<div class="row head">
				<h2>Downloading</h2>
				<button class="btn" onclick={() => downloads.pauseAll()}>Pause all</button>
				<button class="btn" onclick={() => downloads.resumeAll()}>Resume all</button>
			</div>
			<ul class="queue">
				{#each pending as i (i.id)}
					<li>
						<div class="qtext">
							<span class="ellipsis">{i.title || `Track ${i.id}`}</span>
							<span class="muted small">
								{i.status === 'paused' ? 'Paused' : i.status === 'failed' ? i.message : i.status === 'waiting' ? i.message : i.status === 'queued' ? 'Waiting' : i.total ? `${formatBytes(i.received)} of ${formatBytes(i.total)}` : formatBytes(i.received)}
							</span>
							<progress max="1" value={i.total ? i.received / i.total : 0} aria-label={`Progress for ${i.title || `track ${i.id}`}`}></progress>
						</div>
						{#if i.status === 'paused' || i.status === 'failed'}
							<button class="icon-btn" aria-label={i.status === 'failed' ? 'Try again' : 'Resume'} onclick={() => downloads.resume(i.id)}><Icon name="play" size={18} /></button>
						{:else}
							<button class="icon-btn" aria-label="Pause" onclick={() => downloads.pause(i.id)}><Icon name="pause" size={18} /></button>
						{/if}
						<button class="icon-btn" aria-label="Cancel" onclick={() => downloads.cancel(i.id)}><Icon name="close" size={18} /></button>
					</li>
				{/each}
			</ul>
		</div>
	{/if}

	{#if groups.length}
		<ul class="albums">
			{#each groups as g (g.id)}
				<li class="album">
					<div class="ahead">
						<div class="cover"><OfflineCover {user} albumId={g.albumId} label={g.album} /></div>
						<div class="ameta">
							<strong class="ellipsis">{g.album}</strong>
							<span class="ellipsis muted">{g.artist}</span>
							<span class="muted small">{g.tracks.length} {g.tracks.length === 1 ? 'track' : 'tracks'} · {formatBytes(g.size)}</span>
						</div>
						<div class="aact">
							<button class="icon-btn play" aria-label={`Play ${g.album}`} onclick={() => offlinePlayer.playList(user, g.tracks, 0)}><Icon name="play" size={20} /></button>
							<button class="icon-btn" aria-label={`Show tracks of ${g.album}`} aria-expanded={open === g.id} onclick={() => (open = open === g.id ? null : g.id)}><Icon name="chevron" size={20} /></button>
							<button class="icon-btn" aria-label={`Remove ${g.album}`} onclick={() => remove(g.tracks.map((t) => t.key))}><Icon name="trash" size={20} /></button>
						</div>
					</div>
					{#if open === g.id}
						<ol class="tracks">
							{#each g.tracks as t, i (t.key)}
								<li>
									<button class="tmain" onclick={() => offlinePlayer.playList(user, g.tracks, i)} aria-label={`Play ${t.title}`}>
										<span class="num muted">{t.tracknum ?? i + 1}</span>
										<span class="ttext">
											<span class="ellipsis" class:now={offlinePlayer.current?.key === t.key}>{t.title}</span>
											<span class="muted small">{t.format === 'original' ? (t.codec || t.ext).toUpperCase() || FORMAT_LABELS.original : FORMAT_LABELS[t.format]} · {formatBytes(t.size)}{canPlay(t) ? '' : ' · can’t play in this browser'}</span>
										</span>
										<span class="muted small">{duration(t.duration)}</span>
									</button>
									<button class="icon-btn" aria-label={`Remove ${t.title}`} onclick={() => remove([t.key])}><Icon name="trash" size={18} /></button>
								</li>
							{/each}
						</ol>
					{/if}
				</li>
			{/each}
		</ul>
	{:else}
		<p class="muted">Nothing downloaded yet.{offline ? '' : ' Open an album and choose Download to play it without a connection.'}</p>
	{/if}
</section>

<style>
	.downloads {
		display: grid;
		gap: 1rem;
		max-width: 860px;
	}
	h2 {
		margin: 0;
		font-size: 1.1rem;
	}
	.note {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.6rem 0.9rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		margin: 0;
	}
	.storage p {
		margin: 0 0 0.5rem;
	}
	.meter {
		height: 8px;
		border-radius: 99px;
		background: var(--bg-sunken);
		overflow: hidden;
		margin-bottom: 0.5rem;
	}
	.meter span {
		display: block;
		height: 100%;
		background: var(--accent);
	}
	.small {
		font-size: 0.85rem;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		align-items: center;
	}
	.head h2 {
		flex: 1;
	}
	.danger {
		border-color: var(--danger, #c62828);
		color: var(--danger, #c62828);
	}
	.queue,
	.albums,
	.tracks {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.queue {
		margin-top: 0.75rem;
		display: grid;
		gap: 0.5rem;
	}
	.queue li {
		display: flex;
		align-items: center;
		gap: 0.4rem;
	}
	.qtext {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	progress {
		width: 100%;
		height: 6px;
		accent-color: var(--accent);
	}
	.albums {
		display: grid;
		gap: 0.6rem;
	}
	.album {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-raised);
		padding: 0.6rem;
	}
	.ahead {
		display: flex;
		align-items: center;
		gap: 0.8rem;
	}
	.cover {
		width: 64px;
		flex: none;
	}
	.ameta {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.aact {
		display: flex;
		gap: 0.15rem;
	}
	.play {
		background: var(--accent);
		color: var(--bg);
	}
	.tracks {
		margin-top: 0.5rem;
	}
	.tracks li {
		display: flex;
		align-items: center;
	}
	.tmain {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.8rem;
		padding: 0.5rem;
		border: 0;
		background: transparent;
		text-align: left;
		border-radius: var(--radius-sm);
	}
	.tmain:hover {
		background: var(--bg-sunken);
	}
	.num {
		width: 2ch;
		text-align: right;
	}
	.ttext {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.now {
		color: var(--accent);
		font-weight: 600;
	}
</style>
