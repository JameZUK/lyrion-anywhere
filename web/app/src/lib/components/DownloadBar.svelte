<script lang="ts">
	// Download an album or a playlist (SPEC §8), in the page header (Wave 3.5): one Download
	// button in the usual format (Settings › Downloads); "Download as…" and "Remove download"
	// are in the page's ⋯ menu (`actions()`). While it downloads, a line under the header has
	// the progress with Pause and Cancel, "Waiting for Wi-Fi" with "Download now", or the
	// storage limit's refusal. Only library tracks are passed in (never radio or streamed ones).
	import { downloads } from '$lib/offline/index.svelte';
	import type { DownloadRequest } from '$lib/offline/manager.svelte';
	import { refsOf, type Format, type Ref } from '$lib/offline/types';
	import Icon from './Icon.svelte';
	import FormatPicker from './FormatPicker.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import type { RowAction } from './RowMenu.svelte';
	import { controls } from '$lib/player/controls.svelte';

	let {
		tracks,
		ref,
		group,
		name,
		onremoved,
	}: {
		tracks: DownloadRequest[];
		/** `album:<id>` or `playlist:<id>`. */
		ref: Ref;
		/** A playlist's name and order, for the Downloads page. */
		group?: { name: string; ids: number[] };
		/** The album's or playlist's name, for the confirm before removing. */
		name?: string;
		/** Keys of the tracks deleted by "Remove download" (the offline player forgets them). */
		onremoved?: (keys: string[]) => void;
	} = $props();

	let refused = $state('');
	let note = $state('');
	let picking = $state(false);
	/** "Remove download" asked: waiting for a yes. */
	let confirming = $state(false);
	/** A removal is running: no second one starts. */
	let removing = $state(false);
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
	const waiting = $derived(held > 0 && held === progress.count - progress.done - progress.failed - progress.paused);
	/** Downloading, waiting or paused: the line under the header says so. */
	const active = $derived(waiting || progress.busy || progress.paused > 0);
	const server = $derived(controls.needsServer(downloads.online));

	async function download(choice: Format | 'auto' = 'auto') {
		refused = '';
		note = '';
		const r = await downloads.request(tracks, { choice, ref, ...(group ? { group } : {}) });
		refused = r.refused ?? '';
	}

	async function remove() {
		if (removing) return;
		removing = true;
		refused = '';
		note = '';
		try {
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
		} catch {
			refused = 'Couldn’t remove the download. Try again.';
		} finally {
			removing = false;
		}
	}

	const shownName = $derived(name ?? group?.name);
	const confirmTitle = $derived(
		shownName ? `Remove the download of “${shownName}”?` : ref.startsWith('playlist:') ? 'Remove this playlist’s download?' : 'Remove this album’s download?',
	);
	const confirmMessage = $derived(
		`${savedCount} downloaded ${savedCount === 1 ? 'track goes' : 'tracks go'} from this device. Any that another download still needs will stay.`,
	);

	/** For the page's ⋯ menu: Download as… and (last, destructive) Remove download. */
	export function actions(): RowAction[] {
		if (!list.length) return [];
		const out: RowAction[] = [];
		if (!allSaved && !active) out.push({ label: 'Download as…', icon: 'download', run: () => (picking = true), disabled: server.disabled, why: controls.why(server.disabled), id: 'download-as' });
		if (savedCount || removing)
			out.push({ label: 'Remove download', icon: 'trash', run: () => !removing && (confirming = true), disabled: removing, danger: true, id: 'remove-download' });
		return out;
	}
</script>

{#if list.length}
	<div class="dl" data-testid="download-bar">
	{#if allSaved}
		<span class="mark" role="status" data-downloaded><Icon name="downloaded" size={20} /><span class="dl-label">Downloaded</span></span>
	{:else if !active}
		<!-- Fetching needs the server; pausing, cancelling and removing don't. -->
		{@const label = savedCount && savedCount < list.length ? 'Download the rest' : 'Download'}
		<button class="btn dl-btn" onclick={() => download()} {...server} aria-label={label} data-download>
			<Icon name="download" size={18} /><span class="dl-label">{label}</span>
		</button>
	{/if}
	{#if active || refused || note || (progress.failed && !progress.busy) || (savedCount && !allSaved && !active)}
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
			{:else if savedCount && !allSaved}
				<span class="dl-state muted">{savedCount} of {list.length} downloaded</span>
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
	</div>
{/if}
{#if picking}<FormatPicker onchoose={(f) => void download(f)} onclose={() => (picking = false)} />{/if}
{#if confirming}
	<ConfirmDialog
		title={confirmTitle}
		message={confirmMessage}
		confirmLabel="Remove"
		danger
		onconfirm={() => {
			confirming = false;
			void remove();
		}}
		oncancel={() => (confirming = false)}
	/>
{/if}

<style>
	/* Its parts sit in the header's row of actions (the line under it wraps to its own row). */
	.dl {
		display: contents;
	}
	.mark {
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		min-height: 44px;
		padding: 0 0.5rem;
		font-weight: 550;
		color: var(--accent);
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
	/* Phones: Download is an icon beside Play and Shuffle. */
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
