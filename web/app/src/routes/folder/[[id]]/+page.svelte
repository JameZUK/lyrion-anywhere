<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import { useDetailTitle } from '$lib/title.svelte';
	import { page } from '$app/state';
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { folderName, folderRow, rememberName, type FolderRow } from '$lib/lms/folder';
	import { Pager, whenNear } from '$lib/lms/pager.svelte';
	import type { MusicFolderResult } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import Icon from '$lib/components/Icon.svelte';

	const PAGE = 200;
	/** The folder shown: undefined for the top level, null for a bad id. */
	const folder = $derived.by((): number | undefined | null => {
		const raw = page.params.id;
		if (raw === undefined || raw === '') return undefined;
		try {
			return cmd.id(raw);
		} catch {
			return null;
		}
	});
	let sentinel = $state<HTMLDivElement>();
	let ctx = $state<{ kind: cmd.InfoKind; id: number; title: string } | null>(null);

	const query = (f: number | undefined) => async (from: number) => {
		const r = await api.rpc<MusicFolderResult>('', cmd.musicFolder(from, PAGE, f));
		const rows = (r.folder_loop ?? []).map(folderRow).filter((x): x is FolderRow => x !== null);
		rows.forEach(rememberName);
		return { rows, count: r.count };
	};
	const list = new Pager<FolderRow>(query(undefined), (x) => x.id);

	$effect(() => {
		const f = folder;
		void app.libraryVersion;
		if (f === null) return;
		untrack(() => void list.reset(query(f)));
	});
	$effect(() => whenNear(sentinel, () => list.more()));

	const title = $derived(folder === undefined ? 'Music folder' : folder === null ? '' : (folderName(folder) ?? 'Folder'));
	const hasAudio = $derived(list.items.some((x) => x.kind !== 'other'));
	useDetailTitle(() => folder === undefined ? '' : title);
</script>

<section class="page">
	<BackButton parent={folder === undefined ? '/library' : '/folder'} />
	{#if folder === null}
		<p class="error">Not a folder.</p>
	{:else}
		{#if folder !== undefined}<p class="kind muted"><a href="#/folder">Music folder</a></p>{/if}
		<h1>{title}</h1>
		{#if folder !== undefined}
			{@const f = folder}
			<div class="actions">
				<button class="btn primary" onclick={() => app.queueUp({ folder: f }, 'play')} disabled={!hasAudio}><Icon name="play" size={18} />Play all</button>
				<button class="btn" onclick={() => app.queueUp({ folder: f }, 'add')} disabled={!hasAudio}><Icon name="plus" size={18} />Add to queue</button>
				<button class="btn" onclick={() => (ctx = { kind: 'folder', id: f, title })}><Icon name="more" size={18} />More</button>
			</div>
		{/if}
		{#if list.error}<p class="error">{list.error}</p>{/if}
		<ul class="rows">
			{#each list.items as x (x.id)}
				<li>
					{#if x.kind === 'folder'}
						<a class="main" href={cmd.link('folder', x.id) ?? '#/folder'}>
							<span class="glyph" aria-hidden="true"><Icon name="folder" size={20} /></span>
							<span class="name ellipsis">{x.name}</span>
						</a>
						<button class="icon-btn" aria-label={`Play ${x.name}`} onclick={() => app.queueUp({ folder: x.id }, 'play')}><Icon name="play" size={18} /></button>
						<button class="icon-btn" aria-label={`More for ${x.name}`} onclick={() => (ctx = { kind: 'folder', id: x.id, title: x.name })}><Icon name="more" size={18} /></button>
					{:else if x.kind === 'track'}
						<button class="main" onclick={() => app.queueUp({ track: x.id }, 'play')} aria-label={`Play ${x.name}`}>
							<span class="cover"><Art path={x.cover} label={x.name} /></span>
							<span class="name ellipsis">{x.name}</span>
							{#if x.duration}<span class="muted dur">{x.duration}</span>{/if}
						</button>
						<button class="icon-btn" aria-label={`Add ${x.name} to the queue`} onclick={() => app.queueUp({ track: x.id }, 'add')}><Icon name="plus" size={18} /></button>
						<button class="icon-btn" aria-label={`More for ${x.name}`} onclick={() => (ctx = { kind: 'track', id: x.id, title: x.name })}><Icon name="more" size={18} /></button>
					{:else if x.kind === 'playlist'}
						<span class="main">
							<span class="glyph" aria-hidden="true"><Icon name="queue" size={20} /></span>
							<span class="name ellipsis">{x.name}</span>
						</span>
						<button class="icon-btn" aria-label={`Play ${x.name}`} onclick={() => app.queueUp({ playlist: x.id }, 'play')}><Icon name="play" size={18} /></button>
					{:else}
						<span class="main other">
							<span class="glyph" aria-hidden="true"><Icon name="note" size={20} /></span>
							<span class="name ellipsis muted">{x.name}</span>
						</span>
					{/if}
				</li>
			{/each}
		</ul>
		{#if list.total === 0}<p class="muted empty">{folder === undefined ? 'The music folder is empty.' : 'This folder is empty.'}</p>{/if}
		{#if !list.done}<div bind:this={sentinel} aria-hidden="true"></div>{/if}
	{/if}
	{#if ctx}<ContextMenu kind={ctx.kind} id={ctx.id} title={ctx.title} onclose={() => (ctx = null)} />{/if}
</section>

<style>
	.kind {
		text-transform: uppercase;
		letter-spacing: 0.08em;
		font-size: 0.75rem;
		font-weight: 650;
		margin: 0;
	}
	.kind a {
		color: inherit;
		text-decoration: underline;
	}
	h1 {
		font-size: clamp(1.6rem, 3vw, 2.4rem);
		margin: 0.2rem 0 0.8rem;
		overflow-wrap: anywhere;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		margin-bottom: 1.25rem;
	}
	.rows {
		list-style: none;
		padding: 0;
		margin: 0;
		display: grid;
		gap: 0.15rem;
		max-width: 900px;
	}
	li {
		display: flex;
		align-items: center;
		border-radius: var(--radius-sm);
	}
	li:hover {
		background: var(--bg-sunken);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.45rem 0.6rem;
		border: 0;
		background: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.main.other {
		cursor: default;
	}
	span.main {
		cursor: default;
	}
	.glyph,
	.cover {
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
		flex: none;
		overflow: hidden;
	}
	.name {
		font-weight: 550;
		flex: 1;
		min-width: 0;
	}
	.dur {
		font-variant-numeric: tabular-nums;
		font-size: 0.85rem;
	}
	.empty {
		padding: 1.5rem 0;
	}
</style>
