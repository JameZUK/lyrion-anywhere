<script lang="ts">
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, textOf } from '$lib/lms/format';
	import type { PlayerStatus, Track } from '$lib/lms/types';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let tracks = $state<Track[]>([]);
	let total = $state(0);
	let error = $state('');
	let confirmClear = $state(false);

	async function load() {
		const id = app.playerId;
		if (!id) return;
		try {
			const r = await api.rpc<PlayerStatus>(id, cmd.queue(0, 500));
			if (id !== app.playerId) return;
			tracks = r.playlist_loop ?? [];
			total = r.playlist_tracks ?? tracks.length;
			error = '';
		} catch (e) {
			error = (e as Error).message;
		}
	}

	$effect(() => {
		void app.playerId;
		void app.queueVersion;
		untrack(() => void load());
	});

	const current = $derived(Number(app.now?.playlist_cur_index ?? -1));

	async function remove(i: number) {
		tracks = tracks.filter((_, k) => k !== i);
		await app.send(cmd.removeFromQueue(i));
		app.queueVersion++;
	}
	async function move(i: number, by: -1 | 1) {
		const to = i + by;
		if (to < 0 || to >= tracks.length) return;
		const copy = [...tracks];
		[copy[i], copy[to]] = [copy[to]!, copy[i]!];
		tracks = copy;
		await app.send(cmd.moveInQueue(i, to));
		app.queueVersion++;
	}
	async function clear() {
		if (!confirmClear) {
			confirmClear = true;
			setTimeout(() => (confirmClear = false), 4000);
			return;
		}
		confirmClear = false;
		tracks = [];
		await app.send(cmd.clearQueue());
		app.queueVersion++;
	}
</script>

<section class="page">
	<header>
		<div>
			<h1>Queue</h1>
			<p class="muted">{textOf(app.current?.name)}{total ? ` · ${total} tracks` : ''}</p>
		</div>
		{#if tracks.length}
			<button class="btn" class:danger={confirmClear} onclick={clear}><Icon name="trash" size={18} />{confirmClear ? 'Tap again to clear' : 'Clear'}</button>
		{/if}
	</header>
	{#if error}<p class="error">{error}</p>{/if}
	{#if !app.playerId}
		<p class="muted">Choose a player to see its queue.</p>
	{:else if !tracks.length}
		<p class="muted">The queue is empty. <a class="link" href="#/albums">Find something to play</a></p>
	{:else}
		<ol class="q">
			{#each tracks as t, i (`${i}:${t.id}`)}
				<li class:now={i === current}>
					<button class="main" onclick={() => app.send(cmd.jump(i))} aria-label={`Play ${textOf(t.title)}`} aria-current={i === current ? 'true' : undefined}>
						<div class="thumb"><Art path={cmd.coverPath(t.coverid ?? t.artwork_track_id, 96)} alt="" label={textOf(t.title)} /></div>
						<span class="text">
							<span class="title ellipsis">{textOf(t.title)}</span>
							<span class="sub ellipsis muted">{[textOf(t.artist), textOf(t.album)].filter(Boolean).join(' · ')}</span>
						</span>
						<span class="dur muted">{duration(t.duration)}</span>
					</button>
					<div class="ops">
						<button class="icon-btn" aria-label="Move up" onclick={() => move(i, -1)} disabled={i === 0}><Icon name="back" size={16} /></button>
						<button class="icon-btn" aria-label="Move down" onclick={() => move(i, 1)} disabled={i === tracks.length - 1}><Icon name="chevron" size={16} /></button>
						<button class="icon-btn" aria-label={`Remove ${textOf(t.title)}`} onclick={() => remove(i)}><Icon name="close" size={16} /></button>
					</div>
				</li>
			{/each}
		</ol>
		{#if total > tracks.length}<p class="muted">Showing the first {tracks.length} of {total}.</p>{/if}
	{/if}
</section>

<style>
	header {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 1rem;
		margin-bottom: 1rem;
	}
	h1,
	header p {
		margin: 0;
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
	li.now {
		background: var(--accent-soft);
	}
	li.now .title {
		color: var(--accent);
	}
	.main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.8rem;
		padding: 0.45rem 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.thumb {
		width: 44px;
		flex: none;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.title {
		font-weight: 550;
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
	.link {
		text-decoration: underline;
	}
</style>
