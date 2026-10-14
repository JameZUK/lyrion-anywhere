<script lang="ts">
	// What this device has downloaded for the signed-in user (or, offline, the last one):
	// playlists and albums with covers and sizes, storage use (with the limit, if set),
	// downloads in progress or waiting for Wi-Fi, and playback. Removing a playlist or an
	// album keeps the tracks another download still needs.
	import { downloads, offlinePlayer, plays } from '$lib/offline/index.svelte';
	import { formatBytes, loadPrefs, storageInfo, type StorageInfo } from '$lib/offline/settings';
	import { FORMAT_LABELS, refsOf, type StoredTrack } from '$lib/offline/types';
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
	interface PlaylistGroup {
		id: string;
		playlistId: number;
		name: string;
		tracks: StoredTrack[];
		size: number;
	}

	const user = $derived(downloads.user ?? '');
	/** Downloaded playlists, each with its tracks in the playlist's order. */
	const playlists = $derived.by(() => {
		const out: PlaylistGroup[] = [];
		for (const g of downloads.groups) {
			if (!g.id.startsWith('playlist:')) continue;
			const pos = new Map<number, number>();
			g.ids.forEach((id, i) => pos.has(id) || pos.set(id, i));
			const tracks = downloads.saved
				.filter((t) => refsOf(t).includes(g.id))
				.sort((a, b) => (pos.get(a.id) ?? 1e9) - (pos.get(b.id) ?? 1e9) || a.title.localeCompare(b.title));
			if (!tracks.length) continue;
			out.push({ id: g.id, playlistId: Number(g.id.slice(9)), name: g.name || 'Playlist', tracks, size: tracks.reduce((n, t) => n + t.size, 0) });
		}
		return out.sort((a, b) => a.name.localeCompare(b.name));
	});
	/** Albums: tracks downloaded with their album or on their own (not only for a playlist). */
	const groups = $derived.by(() => {
		const map = new Map<string, Group>();
		for (const t of downloads.saved) {
			if (!refsOf(t).some((r) => !r.startsWith('playlist:'))) continue;
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
	const held = $derived(pending.filter((i) => downloads.isHeld(i)).length);
	const cap = $derived.by(() => {
		void downloads.saved.length; // re-read after changes (Settings may have moved it)
		return loadPrefs().capBytes;
	});
	let note = $state('');

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

	function kept(n: number) {
		note = n ? `${n} ${n === 1 ? 'track stays' : 'tracks stay'} on this device: another download still needs ${n === 1 ? 'it' : 'them'}.` : '';
	}
	/** An album (by id) or tracks without one: what they were downloaded for goes. */
	async function removeAlbum(g: Group) {
		const r = g.albumId !== null ? await downloads.removeAlbum(g.albumId) : await downloads.removeTracks(g.tracks.map((t) => t.key));
		offlinePlayer.forget(r.removed);
		kept(r.kept);
	}
	async function removeTrack(t: StoredTrack) {
		const r = await downloads.removeTracks([t.key]);
		offlinePlayer.forget(r.removed);
		kept(r.kept);
	}
	async function removePlaylist(p: PlaylistGroup) {
		const r = await downloads.removePlaylist(p.playlistId);
		offlinePlayer.forget(r.removed);
		kept(r.kept);
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
			{#if cap}<span class="muted" data-testid="storage-limit"> of your {formatBytes(cap)} limit</span>{/if}
			{#if info.quota}<span class="muted"> · this app uses {formatBytes(info.usage ?? 0)} of {formatBytes(info.quota)} available</span>{/if}
		</p>
		{#if plays.pending > 0 && plays.user === downloads.user}
			<p class="muted small" role="status" data-testid="plays-waiting">
				{plays.pending} {plays.pending === 1 ? 'play' : 'plays'} waiting to sync{plays.online ? '' : ': they’ll be sent when you’re back online'}.
			</p>
		{/if}
		{#if info.quota}
			<div class="meter" role="meter" aria-label="Storage used" aria-valuemin="0" aria-valuemax={info.quota} aria-valuenow={info.usage ?? 0}>
				<span style:width={pct(Math.min(1, (info.usage ?? 0) / info.quota))}></span>
			</div>
		{/if}
		<p class="muted small">
			Downloads stay on this device until you remove them, and are kept apart for each account.
			{#if info.persisted === false}The browser may clear them if the device runs short of space.{:else if info.persisted}The browser has agreed to keep them.{/if}
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

	{#if note}<p class="muted small" role="status">{note}</p>{/if}

	{#if pending.length}
		<div class="card">
			{#if held}
				<div class="row wifi" data-testid="wifi-wait">
					<span><Icon name="download" size={18} /> Waiting for Wi-Fi: {held} {held === 1 ? 'download' : 'downloads'} will start on Wi-Fi.</span>
					<button class="btn" onclick={() => downloads.downloadNow()}>Download now</button>
				</div>
			{/if}
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
								{downloads.isHeld(i) ? 'Waiting for Wi-Fi' : i.status === 'paused' ? 'Paused' : i.status === 'failed' ? i.message : i.status === 'waiting' ? i.message : i.status === 'queued' ? 'Waiting' : i.total ? `${formatBytes(i.received)} of ${formatBytes(i.total)}` : formatBytes(i.received)}
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

	{#if playlists.length}
		<h2 class="section">Playlists</h2>
		<ul class="albums" aria-label="Downloaded playlists">
			{#each playlists as p (p.id)}
				<li class="album">
					<div class="ahead">
						<div class="cover"><OfflineCover {user} albumId={p.tracks[0]?.albumId ?? null} label={p.name} /></div>
						<div class="ameta">
							<strong class="ellipsis">{p.name}</strong>
							<span class="ellipsis muted">Playlist</span>
							<span class="muted small">{p.tracks.length} {p.tracks.length === 1 ? 'track' : 'tracks'} · {formatBytes(p.size)}</span>
						</div>
						<div class="aact">
							<button class="icon-btn play" aria-label={`Play ${p.name}`} onclick={() => offlinePlayer.playList(user, p.tracks, 0)}><Icon name="play" size={20} /></button>
							<button class="icon-btn" aria-label={`Show tracks of ${p.name}`} aria-expanded={open === p.id} onclick={() => (open = open === p.id ? null : p.id)}><Icon name="chevron" size={20} /></button>
							<button class="icon-btn" aria-label={`Remove ${p.name}`} onclick={() => removePlaylist(p)}><Icon name="trash" size={20} /></button>
						</div>
					</div>
					{#if open === p.id}
						<ol class="tracks">
							{#each p.tracks as t, i (t.key)}
								<li>
									<button class="tmain" onclick={() => offlinePlayer.playList(user, p.tracks, i)} aria-label={`Play ${t.title}`}>
										<span class="num muted">{i + 1}</span>
										<span class="ttext">
											<span class="ellipsis" class:now={offlinePlayer.current?.key === t.key}>{t.title}</span>
											<span class="muted small">{t.artist} · {formatBytes(t.size)}{canPlay(t) ? '' : ' · can’t play in this browser'}</span>
										</span>
										<span class="muted small">{duration(t.duration)}</span>
									</button>
								</li>
							{/each}
						</ol>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}

	{#if groups.length}
		{#if playlists.length}<h2 class="section">Albums</h2>{/if}
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
							<button class="icon-btn" aria-label={`Remove ${g.album}`} onclick={() => removeAlbum(g)}><Icon name="trash" size={20} /></button>
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
									<button class="icon-btn" aria-label={`Remove ${t.title}`} onclick={() => removeTrack(t)}><Icon name="trash" size={18} /></button>
								</li>
							{/each}
						</ol>
					{/if}
				</li>
			{/each}
		</ul>
	{:else if !playlists.length}
		<p class="muted">Nothing downloaded yet.{offline ? '' : ' Open an album or a playlist and choose Download to play it without a connection.'}</p>
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
	.section {
		margin-top: 0.25rem;
	}
	.wifi {
		padding: 0.5rem 0.75rem;
		margin-bottom: 0.6rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
	}
	.wifi span {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 0.4rem;
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
