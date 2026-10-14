<script lang="ts">
	// "Continue on <player>?" (SPEC §8.4): back online with downloads queued on this device,
	// offer to hand the queue to an LMS player. Never by itself: the user picks the player
	// (this device's own player when it was the one in use, else the last one used) or says
	// "Not now". The choices are only the players this user may control (as Play on… lists
	// them); the hand-off itself is in $lib/offline/handoff.svelte.
	import { api, app } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import { deviceRunning } from '$lib/player/outputs';
	import { handoff, lookupTrack, offlinePlayer } from '$lib/offline/index.svelte';
	import { continueOn, defaultChoice, handoffChoices, handoffText, snapshotOf } from '$lib/offline/handoff.svelte';

	const live = $derived(snapshotOf(offlinePlayer));
	const queue = $derived(handoff.queue(live));
	const choices = $derived(handoffChoices(app.players, { mac: app.deviceMac, supported: device.supported }));
	let picked = $state('');
	const target = $derived(choices.some((c) => c.id === picked) ? picked : defaultChoice(choices, app.playerId));
	let message = $state('');

	// The queue was cleared (or played to its end and stopped) meanwhile: nothing to offer.
	$effect(() => {
		if (handoff.offered && !queue && !handoff.busy) handoff.dismiss();
	});

	async function go() {
		const c = choices.find((x) => x.id === target);
		if (!c || handoff.busy || !queue) return;
		handoff.busy = c.name;
		message = '';
		try {
			if (c.device) {
				// Inside the tap: browsers only let sound start in one.
				if (!deviceRunning()) await device.start();
				const listed = app.players.some((p) => p.playerid === c.id) || (await app.waitForPlayer(c.id));
				if (!listed) {
					message = 'This device couldn’t start playing, so nothing was moved.';
					return;
				}
			}
			const r = await continueOn(c.id, {
				current: () => handoff.queue(snapshotOf(offlinePlayer)),
				lookup: lookupTrack,
				rpc: (p, cmd) => api.rpc(p, cmd),
				stopLocal: () => offlinePlayer.stop(),
			});
			if (r.ok) {
				handoff.dismiss();
				app.select(c.id);
				app.notice = handoffText(r, c.name);
			} else {
				message = handoffText(r, c.name, offlinePlayer.active);
			}
		} catch (e) {
			message = handoffText({ ok: false, reason: 'error', message: (e as Error).message }, c.name, offlinePlayer.active);
		} finally {
			handoff.busy = '';
		}
	}

	const count = $derived(queue?.tracks.length ?? 0);
</script>

{#if handoff.offered && queue && choices.length}
	<div class="offer" role="region" aria-label="Continue on another player">
		<p class="q">
			<span>Back online. Continue on</span>
			<select aria-label="Player" value={target} onchange={(e) => (picked = e.currentTarget.value)} disabled={!!handoff.busy}>
				{#each choices as c (c.id)}
					<option value={c.id}>{c.name}</option>
				{/each}
			</select><span>?</span>
		</p>
		<p class="muted small">{count} {count === 1 ? 'track' : 'tracks'} from your downloads{queue.playing ? ', playing now' : ''}.</p>
		<div class="actions">
			<button class="btn primary" onclick={go} disabled={!!handoff.busy}>{handoff.busy ? `Moving to ${handoff.busy}…` : 'Continue'}</button>
			<button class="btn" onclick={() => handoff.dismiss()} disabled={!!handoff.busy}>Not now</button>
		</div>
		{#if message}<p class="msg" role="status">{message}</p>{/if}
	</div>
{/if}

<style>
	.offer {
		margin: 1rem clamp(1rem, 3vw, 2.5rem) 0;
		padding: 0.6rem 0.9rem;
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		display: grid;
		gap: 0.4rem;
	}
	.q {
		margin: 0;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.4rem;
		font-weight: 600;
	}
	select {
		max-width: 100%;
	}
	.small {
		margin: 0;
		font-size: 0.85rem;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
	.msg {
		margin: 0;
	}
</style>
