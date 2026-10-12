<script lang="ts">
	// A plugin's entry in LMS's home menu (e.g. under My Music), opened as a menu.
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { pluginEntries, toCommand, type PluginEntry, type SBMenu } from '$lib/lms/slimbrowse';
	import MenuView from '$lib/components/MenuView.svelte';

	let entry = $state<PluginEntry | null>(null);
	let status = $state<'loading' | 'ready' | 'missing' | 'error'>('loading');
	let error = $state('');

	async function load(id: string, player: string) {
		status = 'loading';
		try {
			const home = await api.rpc<SBMenu>(player, cmd.homeMenu());
			entry = pluginEntries(home, new Set()).find((e) => e.id === id) ?? null;
			status = entry ? 'ready' : 'missing';
		} catch (e) {
			error = (e as Error).message;
			status = 'error';
		}
	}

	$effect(() => {
		const id = page.params.id ?? '';
		const player = app.playerId ?? '';
		untrack(() => {
			if (player) void load(id, player);
		});
	});

	const start = $derived.by(() => {
		const go = entry?.go;
		return go ? (from: number, count: number) => toCommand(go, { start: from, count }) : undefined;
	});
	const initial = $derived(entry && !entry.go ? { item_loop: entry.children } : undefined);
</script>

<section class="page">
	<a class="back muted" href="#/apps">‹ Apps &amp; Radio</a>
	{#if !app.playerId}
		<p class="muted">Choose a player first: plugins browse and play for a player.</p>
	{:else if status === 'loading'}
		<p class="muted">Loading…</p>
	{:else if status === 'error'}
		<p class="error">{error}</p>
	{:else if status === 'missing' || !entry}
		<p class="muted">This plugin isn’t in the music server’s menu any more.</p>
	{:else}
		{#key entry.id}
			<MenuView {start} {initial} title={entry.text} root={cmd.homeMenu()} />
		{/key}
	{/if}
</section>

<style>
	.back {
		display: inline-block;
		margin-bottom: 0.5rem;
	}
</style>
