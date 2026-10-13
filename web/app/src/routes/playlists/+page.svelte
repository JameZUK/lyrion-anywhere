<script lang="ts">
	import { go } from '$lib/nav';
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { Playlist, PlaylistsResult } from '$lib/lms/types';
	import Icon from '$lib/components/Icon.svelte';

	let lists = $state<Playlist[]>([]);
	let total = $state<number | null>(null);
	let error = $state('');
	let filter = $state('');
	let newName = $state('');
	let creating = $state(false);

	async function load() {
		try {
			const r = await api.rpc<PlaylistsResult>('', cmd.playlists(0, 500));
			lists = r.playlists_loop ?? [];
			total = r.count ?? lists.length;
		} catch (e) {
			error = (e as Error).message;
		}
	}
	$effect(() => {
		void app.libraryVersion;
		untrack(() => void load());
	});

	const shown = $derived(
		filter.trim() ? lists.filter((l) => textOf(l.playlist).toLowerCase().includes(filter.trim().toLowerCase())) : lists,
	);

	async function create(e: SubmitEvent) {
		e.preventDefault();
		try {
			const r = await api.rpc<{ playlist_id?: number }>('', cmd.newPlaylist(newName));
			newName = '';
			creating = false;
			await load();
			if (r.playlist_id) await go(`/playlist/${r.playlist_id}`);
		} catch (err) {
			error = (err as Error).message;
		}
	}
</script>

<section class="page">
	<header>
		<h1>Playlists</h1>
		{#if app.isAdmin}
			<button class="btn" onclick={() => (creating = !creating)} aria-expanded={creating}><Icon name="plus" size={18} />New playlist</button>
		{/if}
	</header>
	{#if creating && app.isAdmin}
		<form class="create" onsubmit={create}>
			<label class="field"><span>Name</span><input bind:value={newName} maxlength="200" required /></label>
			<button class="btn primary" type="submit">Create</button>
		</form>
	{/if}
	{#if error}<p class="error">{error}</p>{/if}
	{#if total !== null && total > 8}
		<label class="filter"><Icon name="search" size={18} /><span class="sr-only">Filter playlists</span><input type="search" bind:value={filter} placeholder="Filter {total} playlists" /></label>
	{/if}
	<ul class="list">
		{#each shown as l (l.id)}
			<li>
				<a class="main" href={cmd.link('playlist', l.id) ?? '#/playlists'}>
					<span class="glyph" aria-hidden="true"><Icon name="queue" size={20} /></span>
					<span class="name ellipsis">{textOf(l.playlist)}</span>
				</a>
				<button class="icon-btn" aria-label={`Play ${textOf(l.playlist)}`} onclick={() => app.queueUp({ playlist: l.id }, 'play')}><Icon name="play" size={18} /></button>
			</li>
		{/each}
	</ul>
	{#if total === 0}<p class="muted">No saved playlists yet.</p>{/if}
</section>

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
	}
	h1 {
		margin: 0;
	}
	.create {
		display: flex;
		align-items: end;
		gap: 0.6rem;
		margin: 1rem 0;
		max-width: 520px;
	}
	.create .field {
		flex: 1;
	}
	.filter {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		max-width: 420px;
		margin: 1rem 0 0;
		padding: 0.45rem 0.8rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
		color: var(--fg-muted);
	}
	.filter input {
		flex: 1;
		border: 0;
		background: transparent;
		outline: none;
		color: var(--fg);
	}
	.list {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
		gap: 0.2rem 1rem;
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
		padding: 0.5rem 0.6rem;
	}
	.glyph {
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent);
		flex: none;
	}
	.name {
		font-weight: 550;
	}
</style>
