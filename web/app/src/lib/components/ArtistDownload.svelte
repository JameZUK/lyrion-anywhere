<script lang="ts">
	// Download all of an artist's albums (SPEC §8.1): each album is its own download (shown
	// as an album on the Downloads page), fetched one album list at a time, checked against
	// the storage limit as one batch.
	import { api } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { isRemote } from '$lib/lms/format';
	import type { Album, TitlesResult } from '$lib/lms/types';
	import { downloads, offlinePlayer } from '$lib/offline/index.svelte';
	import type { DownloadItem, DownloadRequest } from '$lib/offline/manager.svelte';
	import { defaultFormat, networkType } from '$lib/offline/settings';
	import { FORMATS, FORMAT_LABELS, type Format } from '$lib/offline/types';
	import Icon from './Icon.svelte';
	import { controls } from '$lib/player/controls.svelte';

	/** Most albums fetched for one artist download. */
	const MAX_ALBUMS = 200;

	let { albums }: { albums: Album[] } = $props();

	let choice = $state<Format | 'auto'>('auto');
	let busy = $state(false);
	let msg = $state('');
	let note = $state('');
	const auto = defaultFormat();
	const onWifi = networkType() === 'wifi';

	const ids = $derived(new Set(albums.map((a) => a.id).filter((n): n is number => typeof n === 'number' && Number.isSafeInteger(n) && n > 0)));
	const mine = (i: DownloadItem) => (i.albumId !== null && ids.has(i.albumId)) || i.refs.some((r) => r.startsWith('album:') && ids.has(Number(r.slice(6))));
	const progress = $derived(downloads.progress(mine));
	const held = $derived(downloads.items.filter((i) => mine(i) && downloads.isHeld(i)).length);
	const savedAlbums = $derived([...ids].filter((id) => downloads.savedFor(`album:${id}`).length > 0).length);

	async function downloadAll() {
		msg = '';
		note = '';
		busy = true;
		try {
			const all: DownloadRequest[] = [];
			for (const id of [...ids].slice(0, MAX_ALBUMS)) {
				const r = await api.rpc<TitlesResult>('', cmd.albumTracks(id));
				for (const t of r.titles_loop ?? []) {
					if (typeof t.id !== 'number' || isRemote(t.remote)) continue;
					all.push({ id: t.id, albumId: id, duration: t.duration, filesize: t.filesize, ref: `album:${id}` });
				}
			}
			const r = await downloads.request(all, { choice });
			msg = r.refused ?? '';
		} catch (e) {
			msg = (e as Error).message;
		} finally {
			busy = false;
		}
	}

	async function removeAll() {
		msg = '';
		note = '';
		let kept = 0;
		for (const id of ids) {
			const r = await downloads.removeAlbum(id);
			offlinePlayer.forget(r.removed);
			kept += r.kept;
		}
		if (kept) note = `${kept} ${kept === 1 ? 'track stays' : 'tracks stay'}: a downloaded playlist still needs ${kept === 1 ? 'it' : 'them'}.`;
	}
</script>

{#if ids.size}
	<div class="dl" data-testid="artist-download">
		{#if held > 0 && held === progress.count - progress.done - progress.failed - progress.paused}
			<span class="dl-state" role="status"><Icon name="download" size={18} />Waiting for Wi-Fi ({held} {held === 1 ? 'track' : 'tracks'})</span>
			<button class="btn" onclick={() => downloads.downloadNow(mine)} {...controls.needsServer()}>Download now</button>
			<button class="btn" onclick={() => downloads.cancelAll(mine)}><Icon name="close" size={18} />Cancel</button>
		{:else if progress.busy}
			<span class="dl-state" role="status">Downloading {progress.done} of {progress.count} · {Math.round(progress.fraction * 100)}%</span>
			<progress max="1" value={progress.fraction} aria-label="Download progress"></progress>
			<button class="btn" onclick={() => downloads.pauseAll(mine)}><Icon name="pause" size={18} />Pause</button>
			<button class="btn" onclick={() => downloads.cancelAll(mine)}><Icon name="close" size={18} />Cancel</button>
		{:else if progress.paused > 0}
			<span class="dl-state" role="status">Download paused ({progress.done} of {progress.count})</span>
			<button class="btn" onclick={() => downloads.resumeAll(mine)} {...controls.needsServer()}><Icon name="play" size={18} />Resume</button>
			<button class="btn" onclick={() => downloads.cancelAll(mine)}><Icon name="close" size={18} />Cancel</button>
		{:else}
			<!-- Fetching needs the server; pausing, cancelling and removing don't. -->
			<button class="btn" onclick={downloadAll} {...controls.needsServer(downloads.online && !busy)} data-download>
				<Icon name="download" size={18} />{busy ? 'Preparing…' : savedAlbums ? 'Download the other albums' : 'Download all albums'}
			</button>
			<label class="fmt">
				<span class="sr-only">Download format</span>
				<select bind:value={choice} {...controls.needsServer(downloads.online)}>
					<option value="auto">{FORMAT_LABELS[auto]} ({onWifi ? 'Wi-Fi' : 'mobile data'} default)</option>
					{#each FORMATS as f (f)}<option value={f}>{FORMAT_LABELS[f]}</option>{/each}
				</select>
			</label>
			{#if savedAlbums}
				<span class="dl-state muted">{savedAlbums} of {ids.size} {ids.size === 1 ? 'album' : 'albums'} downloaded</span>
				<button class="btn" onclick={removeAll}><Icon name="trash" size={18} />Remove downloads</button>
			{/if}
		{/if}
		{#if msg}<p class="error small" role="alert">{msg}</p>{/if}
		{#if note}<p class="muted small">{note}</p>{/if}
	</div>
{/if}

<style>
	.dl {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
		margin: 0.75rem 0;
	}
	.dl-state {
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		font-weight: 550;
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
</style>
