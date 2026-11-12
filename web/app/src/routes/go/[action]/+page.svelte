<script lang="ts">
	// The installed app's shortcuts (manifest.webmanifest, wave2-design.md §5.5): Resume carries
	// on playing, Radio starts radio from what is playing. Each waits for the player, does its
	// one thing once, then goes to Now playing (or Stations, when there is no radio to start).
	//
	// Anyone can link here, so a page opened from outside the app (the shortcut itself, a link
	// elsewhere, a reload) asks for a tap first, saying what it will do and on which player; it
	// acts at once only when reached from inside the app. Nothing that only shows a page
	// (already playing, nothing to start radio from) needs the tap.
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { untrack } from 'svelte';
	import { api, app } from '#lib/state/app.svelte';
	import * as cmd from '#lib/lms/commands';
	import { isRemote, textOf } from '#lib/lms/format';
	import { radioFromTrack } from '#lib/lms/discovery';
	import { openedFromOutside } from '#lib/nav';

	/** How long to wait for the player before giving up. */
	const WAIT_MS = 20_000;
	const action = $derived(page.params.action ?? '');
	let done = false;
	let status = $state('Getting the player ready…');
	let late = $state(false);
	/** The tap that a page opened from outside waits for. */
	let confirmed = $state(false);

	/** The library track playing now, which radio would start from (streams can't). */
	const track = $derived.by(() => {
		const t = app.now?.playlist_loop?.[0];
		return typeof t?.id === 'number' && t.id > 0 && !isRemote(t.remote) ? { id: t.id, title: textOf(t.title) } : null;
	});
	const ready = $derived(!!app.current && !!app.now);
	/** It would only show a page: nothing to confirm. */
	const showsOnly = $derived((action === 'resume' && app.playing) || (action === 'radio' && !track));
	const ask = $derived(ready && !confirmed && !showsOnly && openedFromOutside(page.url.hash));
	const player = $derived(textOf(app.current?.name) || 'the player');

	async function resume() {
		if (!app.playing) await app.send(cmd.play());
		await goto('#/', { replaceState: true });
	}

	async function radio() {
		const id = track?.id ?? null;
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

	$effect(() => {
		const a = action;
		if (a !== 'resume' && a !== 'radio') {
			void goto('#/', { replaceState: true });
			return;
		}
		if (!ready || done || ask) return;
		done = true;
		untrack(() => void (a === 'resume' ? resume() : radio()));
	});
	$effect(() => {
		const t = setTimeout(() => (late = true), WAIT_MS);
		return () => clearTimeout(t);
	});
</script>

<section class="page" aria-busy={!late && !ask}>
	{#if ask}
		<h1>{action === 'resume' ? 'Resume' : 'Start radio'}</h1>
		<p data-go-ask>
			{#if action === 'resume'}
				Carry on playing on <strong>{player}</strong>?
			{:else}
				Start radio from <strong>{track?.title || 'what is playing'}</strong> on <strong>{player}</strong>?
			{/if}
		</p>
		<div class="actions">
			<button class="btn primary" type="button" onclick={() => (confirmed = true)} data-go-confirm>{action === 'resume' ? 'Resume' : 'Start radio'}</button>
			<a class="btn" href="#/" data-go-cancel>Not now</a>
		</div>
	{:else}
		<p role="status">{late ? 'No player is ready yet.' : status}</p>
		{#if late}<a class="btn" href="#/">Go to Now playing</a>{/if}
	{/if}
</section>

<style>
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
</style>
