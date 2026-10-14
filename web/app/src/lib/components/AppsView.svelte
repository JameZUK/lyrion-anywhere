<script lang="ts">
	// Apps, radio and plugins LMS offers (also a section of Library), found from its own menus.
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { AppInfo, AppsResult } from '$lib/lms/types';
	import { pluginEntries, toCommand, type PluginEntry, type SBMenu, type Suitability } from '$lib/lms/slimbrowse';
	import { probeAll } from '$lib/lms/suitability';
	import Art from './Art.svelte';

	/** Inside Library (a section of it): an h2 title, no page of its own. */
	let { embedded = false }: { embedded?: boolean } = $props();

	let apps = $state<AppInfo[]>([]);
	let radios = $state<AppInfo[]>([]);
	let plugins = $state<PluginEntry[]>([]);
	let error = $state('');
	/** How usable each app/plugin is from here (by `app:cmd` / `plugin:id`), once probed. */
	let fit = $state<Record<string, Suitability>>({});
	let showAll = $state(false);

	// Judge each app and plugin from its own top menu, so ones that are broken or empty here
	// are tucked away and ones needing setup in LMS say so. Needs a player to browse with.
	async function probe(player: string) {
		if (!player) return;
		const targets = [
			...[...radios, ...apps].map((a) => ({ key: `app:${a.cmd}`, command: cmd.appMenu(a.cmd!)(0, 10) })),
			...plugins.flatMap((p) => (p.go ? [{ key: `plugin:${p.id}`, command: toCommand(p.go, { start: 0, count: 10 }) }] : [])),
		];
		await probeAll(targets, (c) => api.rpc<SBMenu>(player, c), (k, s) => (fit = { ...fit, [k]: s }));
	}
	const usable = (key: string) => !['empty', 'error'].includes(fit[key]?.state ?? 'ok');
	const hidden = $derived(
		[...radios, ...apps].filter((a) => !usable(`app:${a.cmd}`)).length + plugins.filter((p) => !usable(`plugin:${p.id}`)).length,
	);
	const setupNote = (key: string) => {
		const s = fit[key];
		return s?.state === 'setup' ? s.message : s?.state === 'error' ? s.message : s?.state === 'empty' ? 'Nothing to show here.' : '';
	};

	async function load() {
		try {
			const [a, r] = await Promise.all([api.rpc<AppsResult>('', cmd.appsList()), api.rpc<AppsResult>('', cmd.radiosList())]);
			const valid = (x: AppInfo) => typeof x.cmd === 'string' && /^[a-z0-9_]{1,64}$/.test(x.cmd);
			apps = (a.appss_loop ?? []).filter(valid);
			radios = (r.radioss_loop ?? []).filter(valid);
		} catch (e) {
			error = (e as Error).message;
		}
	}
	// Plugins that add themselves elsewhere in LMS (My Music, Extras…): from the player's home
	// menu, so whatever is installed shows up, with nothing hard-coded.
	async function loadPlugins(player: string) {
		if (!player) {
			plugins = [];
			return;
		}
		try {
			const home = await api.rpc<SBMenu>(player, cmd.homeMenu());
			const shown = new Set([...apps, ...radios].map((x) => x.cmd!));
			plugins = pluginEntries(home, shown);
		} catch {
			plugins = []; // not essential: apps and radio still work
		}
	}
	$effect(() => {
		void app.libraryVersion;
		const player = app.playerId ?? '';
		untrack(() => void load().then(() => loadPlugins(player)).then(() => probe(player)));
	});
</script>

{#snippet tile(href: string, key: string, icon: unknown, name: string)}
	{@const note = setupNote(key)}
	<li>
		<a {href} class="tile" class:dim={!!note} title={note || undefined}>
			<span class="ic"><Art path={cmd.imagePath(icon, 160)} alt="" label={name} /></span>
			<span class="name ellipsis">{name}</span>
			{#if note}<span class="note muted">{fit[key]?.state === 'setup' ? 'Needs setting up in LMS' : 'Not usable here'}</span>{/if}
		</a>
	</li>
{/snippet}

{#snippet grid(list: AppInfo[])}
	<ul class="grid">
		{#each list.filter((a) => showAll || usable(`app:${a.cmd}`)) as a (a.cmd)}
			{@render tile(`#/browse/${a.cmd}`, `app:${a.cmd}`, a.icon, textOf(a.name))}
		{/each}
	</ul>
{/snippet}

<div class="view" data-view="AppsView">
	{#if embedded}<h2 class="sr-only">Apps &amp; radio</h2>{:else}<h1>Apps &amp; radio</h1>{/if}
	{#if error}<p class="error">{error}</p>{/if}
	{#if !app.playerId}<p class="muted">Tip: choose a player first; some apps need one to browse.</p>{/if}
	{#if radios.length}<h2>Radio</h2>{@render grid(radios)}{/if}
	{#if apps.length}<h2>Apps</h2>{@render grid(apps)}{/if}
	{#if plugins.length}
		<h2>Plugins</h2>
		<ul class="grid">
			{#each plugins.filter((p) => showAll || usable(`plugin:${p.id}`)) as p (p.id)}
				{@render tile(`#/plugin/${encodeURIComponent(p.id)}`, `plugin:${p.id}`, p.icon, p.text)}
			{/each}
		</ul>
	{/if}
	{#if hidden > 0 || showAll}
		<p class="muted hidden-note">
			{#if showAll}Showing everything installed.{:else}{hidden} {hidden === 1 ? 'app is' : 'apps are'} hidden: they show nothing here, or your music server couldn’t open them.{/if}
			<button class="btn" onclick={() => (showAll = !showAll)}>{showAll ? 'Hide unusable' : 'Show all'}</button>
		</p>
	{/if}
	{#if !error && !apps.length && !radios.length && !plugins.length}<p class="muted">No apps installed in LMS.</p>{/if}
</div>

<style>
	h2 {
		margin-top: 1.5rem;
	}
	.grid {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(40vw, 140px), 1fr));
		gap: 1rem;
	}
	.tile {
		display: grid;
		gap: 0.4rem;
		justify-items: center;
		text-align: center;
		padding: 0.75rem;
		border-radius: var(--radius);
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	.tile:hover {
		border-color: var(--accent);
	}
	.ic {
		width: 72px;
	}
	.name {
		font-weight: 600;
		max-width: 100%;
	}
	.dim .ic {
		opacity: 0.55;
	}
	.note {
		font-size: 0.75rem;
	}
	.hidden-note {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.75rem;
		margin-top: 1.5rem;
	}
</style>
