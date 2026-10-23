<script lang="ts">
	// An app's own results for a search ("Search on …" on the Search page), in the plugin menu
	// browser. The app's top menu is read again first, so the search runs through the search
	// box LMS offers now, with the text as its `search:` value and nothing else.
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { app } from '#lib/state/app.svelte';
	import { pluginSearch } from '#lib/state/plugin-search';
	import { cleanQuery, searchCommand, type SearchablePlugin } from '#lib/search/plugins';
	import BackButton from '#lib/components/BackButton.svelte';
	import MenuView from '#lib/components/MenuView.svelte';

	let found = $state<SearchablePlugin | null>(null);
	let status = $state<'loading' | 'ready' | 'missing' | 'error'>('loading');
	let error = $state('');

	const appName = $derived(page.params.app ?? '');
	const query = $derived(cleanQuery(page.params.q ?? ''));

	async function load(player: string, cmd: string) {
		status = 'loading';
		try {
			found = await pluginSearch.find(player, cmd);
			status = found ? 'ready' : 'missing';
		} catch (e) {
			error = (e as Error).message;
			status = 'error';
		}
	}

	$effect(() => {
		const player = app.playerId ?? '';
		const cmd = appName;
		untrack(() => {
			if (player) void load(player, cmd);
		});
	});

	const start = $derived.by(() => {
		const box = found?.search;
		const q = query;
		return box && q ? (from: number, count: number) => searchCommand(box, q, { start: from, count }) : undefined;
	});
</script>

<section class="page">
	<BackButton parent="/search" />
	{#if !query}
		<p class="muted">Type something to search for on the Search page.</p>
	{:else if !app.playerId}
		<p class="muted">No player chosen. Choose a player with Play on: apps search and play for a player.</p>
	{:else if status === 'loading'}
		<p class="muted" role="status">Loading…</p>
	{:else if status === 'error'}
		<p class="error" role="alert">{error}</p>
	{:else if status === 'missing' || !found || !start}
		<p class="muted">This app doesn’t offer a search here any more.</p>
	{:else}
		{#key `${found.cmd}|${query}`}
			<MenuView {start} title={`${found.name}: “${query}”`} />
		{/key}
	{/if}
</section>

<style>
</style>
