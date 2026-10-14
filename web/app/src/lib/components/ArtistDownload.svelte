<script lang="ts">
	// Download all of an artist's albums (SPEC §8.1): each album is its own download (shown
	// as an album on the Downloads page), fetched one album list at a time, checked against
	// the storage limit as one batch. In the page header (Wave 3.5): one Download button in the
	// usual format; "Download all as…" and "Remove downloads" are in the page's ⋯ (`actions()`).
	import { api } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { isRemote } from '$lib/lms/format';
	import type { Album, TitlesResult } from '$lib/lms/types';
	import { downloads, offlinePlayer } from '$lib/offline/index.svelte';
	import type { DownloadItem, DownloadRequest } from '$lib/offline/manager.svelte';
	import type { Format } from '$lib/offline/types';
	import Icon from './Icon.svelte';
	import FormatPicker from './FormatPicker.svelte';
	import type { RowAction } from './RowMenu.svelte';
	import { controls } from '$lib/player/controls.svelte';

	/** Most albums fetched for one artist download. */
	const MAX_ALBUMS = 200;

	let { albums }: { albums: Album[] } = $props();

	let busy = $state(false);
	let msg = $state('');
	let note = $state('');
	let picking = $state(false);

	const ids = $derived(new Set(albums.map((a) => a.id).filter((n): n is number => typeof n === 'number' && Number.isSafeInteger(n) && n > 0)));
	const mine = (i: DownloadItem) => (i.albumId !== null && ids.has(i.albumId)) || i.refs.some((r) => r.startsWith('album:') && ids.has(Number(r.slice(6))));
	const progress = $derived(downloads.progress(mine));
	const held = $derived(downloads.items.filter((i) => mine(i) && downloads.isHeld(i)).length);
	const savedAlbums = $derived([...ids].filter((id) => downloads.savedFor(`album:${id}`).length > 0).length);

	async function downloadAll(choice: Format | 'auto' = 'auto') {
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

	const waiting = $derived(held > 0 && held === progress.count - progress.done - progress.failed - progress.paused);
	const active = $derived(waiting || progress.busy || progress.paused > 0);
	const server = $derived(controls.needsServer(downloads.online && !busy));

	/** For the page's ⋯ menu: Download all as… and (last, destructive) Remove downloads. */
	export function actions(): RowAction[] {
		if (!ids.size) return [];
		const out: RowAction[] = [];
		if (!active) out.push({ label: 'Download all as…', icon: 'download', run: () => (picking = true), disabled: server.disabled, why: controls.why(server.disabled), id: 'download-as' });
		if (savedAlbums) out.push({ label: 'Remove downloads', icon: 'trash', run: () => void removeAll(), danger: true, id: 'remove-download' });
		return out;
	}
</script>

{#if ids.size}
	<div class="dl" data-testid="artist-download">
		{#if !active}
			<!-- Fetching needs the server; pausing, cancelling and removing don't. -->
			<button class="btn dl-btn" onclick={() => downloadAll()} {...server} aria-label={busy ? 'Preparing…' : savedAlbums ? 'Download the other albums' : 'Download all albums'} data-download>
				<Icon name="download" size={18} /><span class="dl-label">{busy ? 'Preparing…' : savedAlbums ? 'Download the other albums' : 'Download all albums'}</span>
			</button>
		{/if}
		{#if active || msg || note || savedAlbums}
			<div class="dl-line">
				{#if waiting}
					<span class="dl-state" role="status">Waiting for Wi-Fi ({held} {held === 1 ? 'track' : 'tracks'})</span>
					<button class="btn" onclick={() => downloads.downloadNow(mine)} {...controls.needsServer()}>Download now</button>
					<button class="btn" onclick={() => downloads.cancelAll(mine)}>Cancel</button>
				{:else if progress.busy}
					<span class="dl-state" role="status">Downloading {progress.done} of {progress.count} · {Math.round(progress.fraction * 100)}%</span>
					<progress max="1" value={progress.fraction} aria-label="Download progress"></progress>
					<button class="btn" onclick={() => downloads.pauseAll(mine)}><Icon name="pause" size={18} />Pause</button>
					<button class="btn" onclick={() => downloads.cancelAll(mine)}>Cancel</button>
				{:else if progress.paused > 0}
					<span class="dl-state" role="status">Download paused ({progress.done} of {progress.count})</span>
					<button class="btn" onclick={() => downloads.resumeAll(mine)} {...controls.needsServer()}><Icon name="play" size={18} />Resume</button>
					<button class="btn" onclick={() => downloads.cancelAll(mine)}>Cancel</button>
				{:else if savedAlbums}
					<span class="dl-state muted">{savedAlbums} of {ids.size} {ids.size === 1 ? 'album' : 'albums'} downloaded</span>
				{/if}
				{#if msg}<p class="error small" role="alert">{msg}</p>{/if}
				{#if note}<p class="muted small">{note}</p>{/if}
			</div>
		{/if}
	</div>
{/if}
{#if picking}<FormatPicker onchoose={(f) => void downloadAll(f)} onclose={() => (picking = false)} />{/if}

<style>
	/* Its parts sit in the header's row of actions (the line under it wraps to its own row). */
	.dl {
		display: contents;
	}
	.dl-line {
		flex-basis: 100%;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
	}
	.dl-state {
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		font-weight: 550;
	}
	.dl-line progress {
		width: 8rem;
		accent-color: var(--accent);
	}
	.small {
		font-size: 0.85rem;
		flex-basis: 100%;
		margin: 0;
	}
	@media (max-width: 480px) {
		.dl-btn {
			padding: 0;
			width: 44px;
			justify-content: center;
		}
		.dl-label {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0 0 0 0);
			white-space: nowrap;
		}
	}
</style>
