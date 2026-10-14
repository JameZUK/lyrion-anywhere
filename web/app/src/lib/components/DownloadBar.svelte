<script lang="ts">
	// Download an album or a playlist (SPEC §8): the format, progress with pause and cancel,
	// "Waiting for Wi-Fi" with "Download now", the storage limit's refusal, and removal. Only
	// library tracks are passed in (never radio or streamed ones).
	import { downloads } from '$lib/offline/index.svelte';
	import type { DownloadRequest } from '$lib/offline/manager.svelte';
	import { defaultFormat, networkType } from '$lib/offline/settings';
	import { FORMATS, FORMAT_LABELS, refsOf, type Format, type Ref } from '$lib/offline/types';
	import Icon from './Icon.svelte';
	import { controls } from '$lib/player/controls.svelte';

	let {
		tracks,
		ref,
		group,
		onremoved,
	}: {
		tracks: DownloadRequest[];
		/** `album:<id>` or `playlist:<id>`. */
		ref: Ref;
		/** A playlist's name and order, for the Downloads page. */
		group?: { name: string; ids: number[] };
		/** Keys of the tracks deleted by "Remove download" (the offline player forgets them). */
		onremoved?: (keys: string[]) => void;
	} = $props();

	let choice = $state<Format | 'auto'>('auto');
	let refused = $state('');
	let note = $state('');
	const auto = defaultFormat();
	const onWifi = networkType() === 'wifi';
	const mine = (i: { refs: string[] }) => i.refs.includes(ref);

	/** Each track once (a playlist may hold one twice). */
	const list = $derived.by(() => {
		const seen = new Set<unknown>();
		return tracks.filter((t) => !seen.has(t.id) && !!seen.add(t.id));
	});
	const savedCount = $derived(list.filter((t) => downloads.savedById(t.id)).length);
	/** Every track is here as part of this album or playlist. */
	const allSaved = $derived(
		list.length > 0 &&
			list.every((t) => {
				const s = downloads.savedById(t.id);
				return !!s && refsOf(s).includes(ref);
			}),
	);
	const progress = $derived(downloads.group(ref));
	const held = $derived(downloads.items.filter((i) => mine(i) && downloads.isHeld(i)).length);
	const firstError = $derived(downloads.items.find((i) => mine(i) && i.status === 'failed')?.message);

	async function download() {
		refused = '';
		note = '';
		const r = await downloads.request(tracks, { choice, ref, ...(group ? { group } : {}) });
		refused = r.refused ?? '';
	}

	async function remove() {
		refused = '';
		const id = Number(ref.slice(ref.indexOf(':') + 1));
		let removed: string[] = [];
		let kept = 0;
		if (ref.startsWith('playlist:')) ({ removed, kept } = await downloads.removePlaylist(id));
		else {
			const a = await downloads.removeAlbum(id);
			// Tracks of this album saved under another album id (a rescan) go too.
			const rest = list.map((t) => downloads.savedById(t.id)?.key).filter((k): k is string => !!k);
			const b = await downloads.removeTracks(rest);
			removed = [...a.removed, ...b.removed];
			kept = b.kept;
		}
		onremoved?.(removed);
		note = kept ? `${kept} ${kept === 1 ? 'track stays' : 'tracks stay'}: another download still needs ${kept === 1 ? 'it' : 'them'}.` : '';
	}
</script>

{#if list.length}
	<div class="dl" data-testid="download-bar">
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
		{:else if allSaved}
			<span class="dl-state done" role="status"><Icon name="check" size={18} />Downloaded</span>
			<button class="btn" onclick={remove}><Icon name="trash" size={18} />Remove download</button>
		{:else}
			<!-- Fetching needs the server; pausing, cancelling and removing don't. -->
			<button class="btn" onclick={download} {...controls.needsServer(downloads.online)} data-download>
				<Icon name="download" size={18} />{savedCount && savedCount < list.length ? 'Download the rest' : 'Download'}
			</button>
			<label class="fmt">
				<span class="sr-only">Download format</span>
				<select bind:value={choice} {...controls.needsServer(downloads.online)}>
					<option value="auto">{FORMAT_LABELS[auto]} ({onWifi ? 'Wi-Fi' : 'mobile data'} default)</option>
					{#each FORMATS as f (f)}<option value={f}>{FORMAT_LABELS[f]}</option>{/each}
				</select>
			</label>
			{#if savedCount}
				<span class="dl-state muted">{savedCount} of {list.length} downloaded</span>
				<button class="btn" onclick={remove}><Icon name="trash" size={18} />Remove download</button>
			{/if}
		{/if}
		{#if refused}<p class="error small" role="alert">{refused}</p>{/if}
		{#if note}<p class="muted small">{note}</p>{/if}
		{#if progress.failed && !progress.busy}
			<p class="error small" role="alert">
				{progress.failed} {progress.failed === 1 ? 'track' : 'tracks'} couldn’t be downloaded: {firstError}
				<button class="btn" onclick={() => downloads.resumeAll(mine)} {...controls.needsServer()}>Try again</button>
			</p>
		{/if}
	</div>
{/if}

<style>
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
</style>
