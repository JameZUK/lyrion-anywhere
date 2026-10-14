<script lang="ts">
	// A page header's actions (Wave 3.5, rule 3): Play, Shuffle, Download where it applies, and
	// ⋯ for the rest (Play next, Add to queue, LMS's own "More…" menu, and the page's own extra
	// actions, destructive ones last). One row on any phone. Everything that sends a command
	// needs the server, and says why it is disabled while it is out of reach.
	import type { Snippet } from 'svelte';
	import { app } from '$lib/state/app.svelte';
	import type { Selection } from '$lib/lms/commands';
	import { controls } from '$lib/player/controls.svelte';
	import Icon from './Icon.svelte';
	import RowMenu, { type RowAction } from './RowMenu.svelte';

	let {
		sel,
		title,
		can = true,
		next = true,
		onmore,
		extra = [],
		download,
	}: {
		/** What Play, Shuffle, Play next and Add to queue act on. */
		sel: Selection;
		/** The page's subject, for the menu's label ("More options for Night Ferry"). */
		title: string;
		/** Anything else the queue actions need (e.g. the playlist has tracks). */
		can?: boolean;
		/** Offer Play next (album, playlist; artists and genres add only). */
		next?: boolean;
		/** LMS's own menu for the item ("More…"). */
		onmore?: () => void;
		/** The page's own actions, after the queue ones. */
		extra?: readonly RowAction[];
		/** The download control (an album's, a playlist's, an artist's). */
		download?: Snippet;
	} = $props();

	const server = $derived(controls.needsServer(can));
	const off = $derived(server.disabled);
	const why = $derived(controls.why(off));
	const actions = $derived.by((): RowAction[] => {
		const out: RowAction[] = [];
		if (next) out.push({ label: 'Play next', icon: 'playnext', run: () => void app.queueUp(sel, 'next'), disabled: off, why, id: 'play-next' });
		out.push({ label: 'Add to queue', icon: 'plus', run: () => void app.queueUp(sel, 'add'), disabled: off, why, id: 'add-to-queue' });
		out.push(...extra);
		// LMS's own menu: it needs the server.
		if (onmore) out.push({ label: 'More…', icon: 'more', run: onmore, disabled: !app.canQueue, why: controls.why(!app.canQueue), id: 'more' });
		return out;
	});
</script>

<div class="page-actions" data-page-actions>
	<button class="btn primary" onclick={() => void app.queueUp(sel, 'play')} {...server} data-page-play><Icon name="play" size={18} />Play</button>
	<button class="btn" onclick={() => void app.shufflePlay(sel)} {...server} data-page-shuffle><Icon name="shuffle" size={18} />Shuffle</button>
	{#if download}{@render download()}{/if}
	<RowMenu label={`More options for ${title || 'this page'}`} {actions} trigger={{ 'data-page-more': true }} />
</div>

<style>
	.page-actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.5rem;
		margin-top: 1rem;
	}
	/* Phones: Play and Shuffle share the row; Download and ⋯ are icons beside them. */
	@media (max-width: 480px) {
		.page-actions > .btn {
			flex: 1 1 0;
			justify-content: center;
			min-width: 0;
		}
	}
</style>
