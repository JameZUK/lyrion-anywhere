<script lang="ts">
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { Artist } from '$lib/lms/types';

	let { artists, offset = 0 }: { artists: Artist[]; /** Where the first artist sits in the whole list. */ offset?: number } = $props();
</script>

<ul class="list">
	{#each artists as a, i (a.id)}
		<li data-pos={offset + i}><a href={cmd.link('artist', a.id) ?? '#/artists'}><span class="avatar" aria-hidden="true">{textOf(a.artist).charAt(0).toUpperCase()}</span><span class="ellipsis">{textOf(a.artist)}</span></a></li>
	{/each}
</ul>

<style>
	.list {
		list-style: none;
		padding: 0;
		margin: 1rem 0 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 240px), 1fr));
		gap: 0.25rem 1rem;
	}
	a {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 44px;
		box-sizing: border-box;
		padding: 0.45rem 0.5rem;
		border-radius: var(--radius-sm);
		min-width: 0;
	}
	a:hover {
		background: var(--bg-sunken);
	}
	.avatar {
		flex: none;
		width: 36px;
		height: 36px;
		border-radius: 50%;
		display: grid;
		place-items: center;
		background: var(--accent-soft);
		color: var(--accent);
		font-weight: 700;
	}
</style>
