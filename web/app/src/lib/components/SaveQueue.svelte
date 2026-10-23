<script lang="ts">
	// "Save as playlist" for the selected player's queue: admins only (saved playlists are
	// shared household data), and only when the gateway offers it. Opens SaveQueueDialog; once
	// saved, says so with a link to the playlist: through `onsaved` when the page has its own
	// message bar, else in its own.
	import { app } from '#lib/state/app.svelte';
	import { loadFeatures, serverFeatures } from '#lib/state/features.svelte';
	import type { SavedPlaylist } from '#lib/api/me';
	import { suggestPlaylistName } from '#lib/queue/save-name';
	import * as cmd from '#lib/lms/commands';
	import { go } from '#lib/nav';
	import Icon from './Icon.svelte';
	import SaveQueueDialog from './SaveQueueDialog.svelte';
	import Snackbar from './Snackbar.svelte';

	let {
		tracks = [],
		full = false,
		button = true,
		onsaved,
		onopen,
	}: {
		/** The queue as shown (for the suggested name). */
		tracks?: readonly { album?: unknown }[];
		/** A full-width button (in a sheet or side panel). */
		full?: boolean;
		/** Its own button; without one, a menu calls `start()` when `available()`. */
		button?: boolean;
		/** Say it was saved (the caller shows its own message); otherwise shown here. */
		onsaved?: (p: SavedPlaylist) => void;
		/** The playlist is about to be opened (a sheet can close itself). */
		onopen?: () => void;
	} = $props();

	let open = $state(false);
	let saved = $state<SavedPlaylist | null>(null);
	let suggestion = $state('');

	$effect(() => {
		if (app.isAdmin) loadFeatures();
	});

	const shown = $derived(app.isAdmin && serverFeatures.saveQueue && !!app.playerId);

	/** Offered here: admins only, and only when the gateway can. */
	export function available(): boolean {
		return shown;
	}

	export function start() {
		suggestion = suggestPlaylistName(tracks);
		open = true;
	}

	function done(p: SavedPlaylist) {
		open = false;
		if (onsaved) onsaved(p);
		else saved = p;
	}

	function openPlaylist() {
		const p = saved;
		saved = null;
		const to = p ? cmd.link('playlist', p.id) : null;
		if (!to) return;
		onopen?.();
		void go(to);
	}
</script>

{#if shown && button}
	<button class="btn" class:full onclick={start} aria-haspopup="dialog" data-save-queue><Icon name="plus" size={18} />Save as playlist</button>
{/if}
{#if open && app.playerId}
	<SaveQueueDialog player={app.playerId} {suggestion} onsaved={done} oncancel={() => (open = false)} />
{/if}
{#if !onsaved}
	<Snackbar message={saved ? `Saved as “${saved.name}”` : ''} action={saved ? 'Open' : ''} onaction={openPlaylist} ondismiss={() => (saved = null)} />
{/if}

<style>
	.full {
		width: 100%;
		margin-top: 0.5rem;
		justify-content: center;
	}
</style>
