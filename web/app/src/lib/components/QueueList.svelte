<script lang="ts">
	// The current track and what follows (Wave 2 §3.4), in sections when this device can vouch
	// for its Up next: in the queue peek's sheet, and beside Now playing in wide windows (§4.2).
	// Tap a track to play it; the full queue (to edit it) is a link away. Admins can save the
	// queue as a playlist from here too.
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, textOf } from '$lib/lms/format';
	import { trackerVersion, upNextTracker } from '$lib/queue/upnext.svelte';
	import type { PlayerStatus, Track } from '$lib/lms/types';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import SaveQueue from './SaveQueue.svelte';
	import { controls, local } from '$lib/player/controls.svelte';
	import { MARK_LABELS } from '$lib/player/local-plan';
	import { isNetworkError } from '$lib/state/reachability';

	let { onplay, side = false }: { onplay?: () => void; side?: boolean } = $props();
	const uid = `ql-${Math.random().toString(36).slice(2, 9)}`;

	/** How many tracks the peek shows. */
	const PEEK = 30;
	let rows = $state<Track[]>([]);
	let from = $state(0);
	let error = $state('');
	let loaded = $state(false);

	$effect(() => {
		const id = app.playerId;
		void app.queueVersion;
		if (!id) return;
		const cur = Math.max(0, Number(app.now?.playlist_cur_index ?? 0) || 0);
		let live = true;
		api.rpc<PlayerStatus>(id, cmd.queue(cur, PEEK)).then(
			(r) => {
				if (!live) return;
				rows = r?.playlist_loop ?? [];
				from = cur;
				loaded = true;
				error = '';
				upNextTracker.verify(id, cur, [...Array<null>(cur).fill(null), ...rows.map((t) => (typeof t.id === 'number' ? t.id : null))]);
			},
			// Out of reach: the connection message says so; the rows as last read stay.
			(e) => live && (error = isNetworkError(e) || app.reach !== 'online' ? '' : (e as Error).message),
		);
		return () => (live = false);
	});

	const sections = $derived.by(() => {
		void trackerVersion();
		return app.playerId ? upNextTracker.sections(app.playerId, from, from + rows.length) : null;
	});
	type Group = { key: string; title: string; start: number; end: number };
	const groups = $derived.by((): Group[] => {
		const n = rows.length;
		if (!n) return [];
		if (!sections) return [{ key: 'now', title: 'Now playing', start: 0, end: 0 }, ...(n > 1 ? [{ key: 'then', title: 'Next', start: 1, end: n - 1 }] : [])];
		const out: Group[] = [{ key: 'now', title: 'Now playing', start: 0, end: 0 }];
		out.push({ key: 'next', title: 'Up next', start: sections.from - from, end: sections.to - from });
		if (sections.to - from + 1 < n) out.push({ key: 'then', title: 'Then', start: sections.to - from + 1, end: n - 1 });
		return out;
	});
	const total = $derived(Number(app.now?.playlist_tracks ?? 0));
	/** Which controls work now: out of reach, only rows this device can play. */
	const g = $derived(controls.gates);
	/** The row playing: the first, unless this device has moved on by itself meanwhile. */
	const nowRow = $derived(local.engaged && local.index >= 0 ? local.index - from : 0);

	function play(i: number) {
		if (!g.jump(from + i)) return;
		controls.jump(from + i);
		onplay?.();
	}
</script>

{#if error}<p class="error">{error}</p>{/if}
{#if loaded && !rows.length}<p class="muted">The queue is empty.</p>{/if}
{#each groups as grp (grp.key)}
	<h3 id={`${uid}-${grp.key}`}>{grp.title}</h3>
	<ol aria-labelledby={`${uid}-${grp.key}`} data-peek={side ? undefined : grp.key} data-side-queue={side ? grp.key : undefined}>
		{#each rows.slice(grp.start, grp.end + 1) as t, k (`${grp.start + k}:${t.id}`)}
			{@const i = grp.start + k}
			{@const mark = controls.mark(from + i, t.id)}
			{@const playable = g.jump(from + i)}
			<li class:unavailable={!playable}>
				<button
					onclick={() => play(i)}
					disabled={!playable}
					aria-current={i === nowRow ? 'true' : undefined}
					aria-label={`Play ${textOf(t.title)}${t.artist ? ` by ${textOf(t.artist)}` : ''}${mark ? `. ${MARK_LABELS[mark]}` : ''}`}
					data-mark={mark ?? undefined}
				>
					<span class="thumb"><Art path={cmd.coverPath(t.coverid ?? t.artwork_track_id, 96)} alt="" label={textOf(t.title)} /></span>
					<span class="text">
						<span class="title ellipsis">{textOf(t.title)}</span>
						<span class="sub ellipsis muted">{textOf(t.artist)}</span>
					</span>
					{#if mark}<span class="mark" title={MARK_LABELS[mark]} data-queue-mark={mark}><Icon name={mark === 'downloaded' ? 'download' : 'device'} size={15} /></span>{/if}
					<span class="dur muted">{duration(t.duration)}</span>
				</button>
			</li>
		{/each}
	</ol>
{/each}
{#if total > from + rows.length}<p class="muted more">And {(total - from - rows.length).toLocaleString('en-GB')} more.</p>{/if}
<a class="btn full" href="#/queue" onclick={() => onplay?.()} data-full-queue>Open the full queue</a>
{#if rows.length && app.reach === 'online'}<SaveQueue tracks={rows} full onopen={onplay} />{/if}

<style>
	h3 {
		margin: 0.75rem 0 0.3rem;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
	}
	ol {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	li button {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 0.7rem;
		min-height: 44px;
		padding: 0.35rem 0.4rem;
		border: 0;
		background: transparent;
		text-align: left;
		color: var(--fg);
		font: inherit;
		border-radius: var(--radius-sm);
	}
	li button:hover:not(:disabled) {
		background: var(--bg-sunken);
	}
	/* Out of reach: rows this device can't play are dimmed, and can't be tapped. */
	li.unavailable button {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.mark {
		flex: none;
		display: inline-grid;
		place-items: center;
		color: var(--accent);
	}
	li button[aria-current='true'] .title {
		color: var(--accent);
	}
	.thumb {
		width: 40px;
		flex: none;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.sub,
	.dur {
		font-size: 0.85rem;
	}
	.more {
		font-size: 0.85rem;
	}
	.full {
		margin-top: 0.75rem;
	}
</style>
