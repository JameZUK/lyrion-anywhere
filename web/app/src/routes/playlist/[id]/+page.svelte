<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import { useDetailTitle } from '$lib/title.svelte';
	import { go } from '$lib/nav';
	import { page } from '$app/state';
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, isRemote, textOf } from '$lib/lms/format';
	import type { PlaylistTracksResult, Playlist, PlaylistsResult, Track } from '$lib/lms/types';
	import type { Command } from '$lib/api/rpc';
	import Icon from '$lib/components/Icon.svelte';
	import DownloadBar from '$lib/components/DownloadBar.svelte';
	import { downloads, offlinePlayer } from '$lib/offline/index.svelte';

	let info = $state<Playlist | null>(null);
	let tracks = $state<Track[]>([]);
	let error = $state('');
	let renaming = $state(false);
	let newName = $state('');
	let confirmDelete = $state(false);

	const listId = $derived.by(() => {
		try {
			return cmd.id(page.params.id);
		} catch {
			return null;
		}
	});

	async function load(id: number) {
		try {
			const [all, t] = await Promise.all([
				api.rpc<PlaylistsResult>('', cmd.playlists(0, 500)),
				api.rpc<PlaylistTracksResult>('', cmd.playlistTracks(id)),
			]);
			info = all.playlists_loop?.find((p) => p.id === id) ?? null;
			tracks = t.playlisttracks_loop ?? [];
			if (!info) error = 'Playlist not found.';
		} catch (e) {
			error = (e as Error).message;
		}
	}

	$effect(() => {
		const id = listId;
		untrack(() => {
			info = null;
			tracks = [];
			error = id === null ? 'Not a playlist.' : '';
			if (id !== null) void load(id);
		});
	});

	async function edit(c: Command, id: number) {
		try {
			await api.rpc('', c);
		} catch (e) {
			error = (e as Error).message;
		}
		await load(id);
	}
	async function rename(e: SubmitEvent, id: number) {
		e.preventDefault();
		renaming = false;
		await edit(cmd.renamePlaylist(id, newName), id);
	}
	async function remove(id: number) {
		if (!confirmDelete) {
			confirmDelete = true;
			setTimeout(() => (confirmDelete = false), 4000);
			return;
		}
		try {
			await api.rpc('', cmd.deletePlaylist(id));
			await go('/playlists');
		} catch (e) {
			error = (e as Error).message;
		}
	}
	// Downloads (SPEC §8.1): the playlist's library tracks, as a group on the Downloads page.
	const requests = $derived(
		tracks
			.filter((t) => typeof t.id === 'number' && t.id > 0 && !isRemote(t.remote))
			.map((t) => ({ id: t.id, albumId: t.album_id, duration: t.duration, filesize: t.filesize })),
	);
	const order = $derived(requests.map((t) => t.id as number));
	const totalTime = $derived(tracks.reduce((s, t) => s + (Number(t.duration) || 0), 0));
	useDetailTitle(() => textOf(info?.playlist));
</script>

<section class="page">
	<BackButton parent="/playlists" />
	{#if error}<p class="error">{error}</p>{/if}
	{#if info && listId !== null}
		{@const id = listId}
		<header>
			<p class="kind muted">Playlist</p>
			{#if renaming && app.isAdmin}
				<form class="rename" onsubmit={(e) => rename(e, id)}>
					<label class="field"><span class="sr-only">New name</span><input bind:value={newName} maxlength="200" required /></label>
					<button class="btn primary" type="submit">Save</button>
					<button class="btn" type="button" onclick={() => (renaming = false)}>Cancel</button>
				</form>
			{:else}
				<h1>{textOf(info.playlist)}</h1>
			{/if}
			<p class="muted">{tracks.length} tracks · {duration(totalTime)}</p>
			<div class="actions">
				<button class="btn primary" onclick={() => app.queueUp({ playlist: id }, 'play')} disabled={!tracks.length || !app.canQueue}><Icon name="play" size={18} />Play</button>
				<button class="btn" onclick={() => app.queueUp({ playlist: id }, 'next')} disabled={!tracks.length || !app.canQueue}><Icon name="playnext" size={18} />Play next</button>
				<button class="btn" onclick={() => app.queueUp({ playlist: id }, 'add')} disabled={!tracks.length || !app.canQueue}><Icon name="plus" size={18} />Add to queue</button>
				{#if app.isAdmin}
					<button class="btn" onclick={() => { newName = textOf(info?.playlist); renaming = true; }}>Rename</button>
					<button class="btn" class:danger={confirmDelete} onclick={() => remove(id)}><Icon name="trash" size={18} />{confirmDelete ? 'Tap again to delete' : 'Delete'}</button>
				{/if}
			</div>
			<DownloadBar
				tracks={requests}
				ref={`playlist:${id}`}
				group={{ name: textOf(info.playlist) || 'Playlist', ids: order }}
				onremoved={(keys) => offlinePlayer.forget(keys)}
			/>
		</header>
		<ol class="q">
			{#each tracks as t, i (`${i}:${t.id}`)}
				<li>
					<button class="main" onclick={() => app.queueUp({ playlist: id }, 'play', i)} disabled={!app.canQueue} aria-label={`Play from ${textOf(t.title)}`}>
						<span class="num muted">{i + 1}</span>
						<span class="text">
							<span class="title ellipsis">{textOf(t.title)}</span>
							<span class="sub ellipsis muted">{[textOf(t.artist), textOf(t.album)].filter(Boolean).join(' · ')}</span>
						</span>
						{#if downloads.savedById(t.id)}<span class="saved" title="Downloaded"><Icon name="check" size={16} label="Downloaded" /></span>{/if}
						<span class="dur muted">{duration(t.duration)}</span>
					</button>
					{#if app.isAdmin}
					<div class="ops">
						<button class="icon-btn" aria-label="Move up" disabled={i === 0} onclick={() => edit(cmd.moveInPlaylist(id, i, i - 1), id)}><Icon name="back" size={16} /></button>
						<button class="icon-btn" aria-label="Move down" disabled={i === tracks.length - 1} onclick={() => edit(cmd.moveInPlaylist(id, i, i + 1), id)}><Icon name="chevron" size={16} /></button>
						<button class="icon-btn" aria-label={`Remove ${textOf(t.title)} from the playlist`} onclick={() => edit(cmd.removeFromPlaylist(id, i), id)}><Icon name="close" size={16} /></button>
					</div>
					{/if}
				</li>
			{/each}
		</ol>
	{:else if !error}
		<p class="muted">Loading…</p>
	{/if}
</section>

<style>
	header {
		margin: 0.75rem 0 1.5rem;
	}
	header p {
		margin: 0.2rem 0;
	}
	.kind {
		text-transform: uppercase;
		letter-spacing: 0.08em;
		font-size: 0.75rem;
		font-weight: 650;
	}
	h1 {
		font-size: clamp(1.6rem, 3vw, 2.4rem);
		margin: 0.2rem 0;
	}
	.rename {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		max-width: 560px;
	}
	.rename .field {
		flex: 1;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		margin-top: 1rem;
	}
	.danger {
		border-color: var(--danger);
		color: var(--danger);
	}
	.q {
		list-style: none;
		margin: 0;
		padding: 0;
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
		gap: 0.9rem;
		padding: 0.5rem 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.saved {
		display: inline-flex;
		color: var(--accent);
	}
	.num {
		width: 3ch;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.sub {
		font-size: 0.85rem;
	}
	.dur {
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.ops {
		display: flex;
		opacity: 0.55;
	}
	li:hover .ops,
	.ops:focus-within {
		opacity: 1;
	}
	.ops .icon-btn:first-child :global(svg) {
		transform: rotate(90deg);
	}
</style>
