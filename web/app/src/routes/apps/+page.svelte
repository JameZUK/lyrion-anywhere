<script lang="ts">
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { AppInfo, AppsResult } from '$lib/lms/types';
	import { pluginEntries, type PluginEntry, type SBMenu } from '$lib/lms/slimbrowse';
	import Art from '$lib/components/Art.svelte';

	let apps = $state<AppInfo[]>([]);
	let radios = $state<AppInfo[]>([]);
	let plugins = $state<PluginEntry[]>([]);
	let error = $state('');

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
		untrack(() => void load().then(() => loadPlugins(player)));
	});
</script>

{#snippet grid(list: AppInfo[])}
	<ul class="grid">
		{#each list as a (a.cmd)}
			<li>
				<a href={`#/browse/${a.cmd}`} class="tile">
					<span class="ic"><Art path={cmd.imagePath(a.icon, 160)} alt="" label={textOf(a.name)} /></span>
					<span class="name ellipsis">{textOf(a.name)}</span>
				</a>
			</li>
		{/each}
	</ul>
{/snippet}

<section class="page">
	<h1>Apps &amp; Radio</h1>
	{#if error}<p class="error">{error}</p>{/if}
	{#if !app.playerId}<p class="muted">Tip: choose a player first; some apps need one to browse.</p>{/if}
	{#if radios.length}<h2>Radio</h2>{@render grid(radios)}{/if}
	{#if apps.length}<h2>Apps</h2>{@render grid(apps)}{/if}
	{#if plugins.length}
		<h2>Plugins</h2>
		<ul class="grid">
			{#each plugins as p (p.id)}
				<li>
					<a href={`#/plugin/${encodeURIComponent(p.id)}`} class="tile">
						<span class="ic"><Art path={cmd.imagePath(p.icon, 160)} alt="" label={p.text} /></span>
						<span class="name ellipsis">{p.text}</span>
					</a>
				</li>
			{/each}
		</ul>
	{/if}
	{#if !error && !apps.length && !radios.length && !plugins.length}<p class="muted">No apps installed in LMS.</p>{/if}
</section>

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
</style>
