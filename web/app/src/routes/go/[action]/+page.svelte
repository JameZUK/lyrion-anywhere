<script lang="ts">
	// The installed app's shortcuts (manifest.webmanifest, wave2-design.md §5.5): Resume carries
	// on playing, Radio starts radio from what is playing. Each waits for the player, does its
	// one thing once, then goes to Now playing (or Stations, when there is no radio to start).
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { untrack } from 'svelte';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { isRemote } from '$lib/lms/format';
	import { radioFromTrack } from '$lib/lms/discovery';

	/** How long to wait for the player before giving up. */
	const WAIT_MS = 20_000;
	const action = $derived(page.params.action ?? '');
	let done = false;
	let status = $state('Getting the player ready…');
	let late = $state(false);

	async function resume() {
		if (!app.playing) await app.send(cmd.play());
		await goto('#/', { replaceState: true });
	}

	async function radio() {
		const t = app.now?.playlist_loop?.[0];
		const id = typeof t?.id === 'number' && t.id > 0 && !isRemote(t.remote) ? t.id : null;
		if (id === null) {
			app.notice = 'Nothing from your library is playing to start radio from. Choose a station instead.';
			await goto('#/stations', { replaceState: true });
			return;
		}
		status = 'Starting radio…';
		const msg = await radioFromTrack((p, c) => api.rpc(p, c), app.playerId, id);
		app.soon();
		if (msg) {
			app.notice = msg;
			await goto('#/stations', { replaceState: true });
		} else {
			await goto('#/', { replaceState: true });
		}
	}

	const ready = $derived(!!app.current && !!app.now);
	$effect(() => {
		const a = action;
		if (a !== 'resume' && a !== 'radio') {
			void goto('#/', { replaceState: true });
			return;
		}
		if (!ready || done) return;
		done = true;
		untrack(() => void (a === 'resume' ? resume() : radio()));
	});
	$effect(() => {
		const t = setTimeout(() => (late = true), WAIT_MS);
		return () => clearTimeout(t);
	});
</script>

<section class="page" aria-busy={!late}>
	<p role="status">{late ? 'No player is ready yet.' : status}</p>
	{#if late}<a class="btn" href="#/">Go to Now playing</a>{/if}
</section>
