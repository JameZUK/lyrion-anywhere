<script lang="ts">
	// Settings › Listening history: on or off, view it, clear it (docs/PRIVACY.md).
	import { api } from '$lib/state/app.svelte';
	import { me } from '$lib/state/me';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import type { HistoryEntry } from '$lib/api/me';
	import type { TitlesResult, Track } from '$lib/lms/types';

	let enabled = $state<boolean | null>(null);
	let days = $state(396);
	let items = $state<HistoryEntry[]>([]);
	let tracks = $state<Map<number, Track>>(new Map());
	let next = $state<string | null>(null);
	let open = $state(false);
	let message = $state('');
	let busy = $state(false);

	async function loadPage(before?: string | null) {
		const page = await me.history(50, before);
		enabled = page.enabled;
		days = page.retention_days;
		next = page.next;
		return page.items;
	}

	async function start() {
		try {
			await loadPage();
		} catch (e) {
			message = (e as Error).message;
		}
	}
	void start();

	/** Titles for the plays shown (ids that no longer resolve show as an unknown track). */
	async function describe(list: HistoryEntry[]) {
		const ids = [...new Set(list.map((e) => e.track_id))].filter((id) => !tracks.has(id));
		const map = new Map(tracks);
		for (const part of cmd.chunkIds(ids)) {
			try {
				const r = await api.rpc<TitlesResult>('', cmd.tracksByIds(part));
				for (const t of r.titles_loop ?? []) if (typeof t.id === 'number') map.set(t.id, t);
			} catch {
				/* show what we have */
			}
		}
		tracks = map;
	}

	async function show() {
		open = true;
		busy = true;
		try {
			items = await loadPage();
			await describe(items);
		} catch (e) {
			message = (e as Error).message;
		} finally {
			busy = false;
		}
	}

	async function more() {
		if (!next) return;
		busy = true;
		try {
			const page = await loadPage(next);
			items = [...items, ...page];
			await describe(page);
		} catch (e) {
			message = (e as Error).message;
		} finally {
			busy = false;
		}
	}

	async function toggle() {
		const want = !enabled;
		try {
			enabled = (await me.setHistory(want)).enabled;
			message = want ? 'Listening history is on.' : 'Listening history is off: nothing more is recorded.';
		} catch (e) {
			message = (e as Error).message;
		}
	}

	async function clear() {
		if (!confirm('Clear your listening history? This can’t be undone.')) return;
		try {
			const r = await me.clearHistory();
			items = [];
			next = null;
			message = `Cleared ${r.removed} ${r.removed === 1 ? 'play' : 'plays'}.`;
		} catch (e) {
			message = (e as Error).message;
		}
	}

	const when = (t: number) => new Date(t * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
	const months = $derived(Math.round(days / 30.4));
</script>

<p class="muted">
	What you play here, on this device or offline, is remembered for your Home shelves and stations: which track, when, and for how long. It’s kept for {months} months, only for you, and never shared with
	other users.
</p>
{#if enabled !== null}
	<label class="toggle">
		<input type="checkbox" checked={enabled} onchange={toggle} />
		Keep my listening history
	</label>
{/if}
<div class="buttons">
	<button class="btn" onclick={show} disabled={busy}>View history</button>
	<button class="btn" onclick={clear}>Clear history</button>
</div>
{#if message}<p class="note" role="status">{message}</p>{/if}
{#if open}
	{#if items.length}
		<ol class="history" aria-label="Listening history">
			{#each items as e, i (`${e.played_at}-${e.track_key}-${i}`)}
				{@const t = tracks.get(e.track_id)}
				<li>
					<span class="what ellipsis">{t ? `${textOf(t.title)} · ${textOf(t.artist)}` : 'A track no longer in the library'}</span>
					<span class="muted small">{when(e.played_at)}{e.source === 'offline' ? ' · offline' : ''}</span>
				</li>
			{/each}
		</ol>
		{#if next}<button class="btn" onclick={more} disabled={busy}>Show older</button>{/if}
	{:else if !busy}
		<p class="muted">Nothing recorded yet.</p>
	{/if}
{/if}

<style>
	.toggle {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin: 0.5rem 0;
	}
	.buttons {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	.note {
		padding: 0.4rem 0.7rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
	}
	.history {
		list-style: none;
		margin: 0.75rem 0;
		padding: 0;
		display: grid;
		gap: 0.3rem;
		max-height: 24rem;
		overflow-y: auto;
	}
	.history li {
		display: grid;
		gap: 0.05rem;
		min-width: 0;
	}
	.small {
		font-size: 0.82rem;
	}
</style>
