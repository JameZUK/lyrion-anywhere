<script lang="ts">
	// "Continue on <player>?" (SPEC §8.4): back online with downloads queued on this device,
	// offer to hand the queue to an LMS player. Never by itself: the user picks the player
	// (this device's own player when it was the one in use, else the last one used) or says
	// "Not now". Wave 3.5: an offer is a snackbar with an action ("Continue"), and a second
	// one ("Other player…") for the list of players. The choices are only the players this
	// user may control (as Play on… lists them); the hand-off itself is in
	// $lib/offline/handoff.svelte.
	import { api, app } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import { deviceRunning } from '$lib/player/outputs';
	import { handoff, lookupTrack, offlinePlayer } from '$lib/offline/index.svelte';
	import { continueOn, defaultChoice, handoffChoices, handoffText, snapshotOf } from '$lib/offline/handoff.svelte';
	import Icon from './Icon.svelte';
	import Sheet from './Sheet.svelte';
	import Snackbar from './Snackbar.svelte';

	const live = $derived(snapshotOf(offlinePlayer));
	const queue = $derived(handoff.queue(live));
	const choices = $derived(handoffChoices(app.players, { mac: app.deviceMac, supported: device.supported }));
	let picked = $state('');
	const target = $derived(choices.some((c) => c.id === picked) ? picked : defaultChoice(choices, app.playerId));
	const targetName = $derived(choices.find((c) => c.id === target)?.name ?? '');
	let message = $state('');
	let choosing = $state(false);

	// The queue was cleared (or played to its end and stopped) meanwhile: nothing to offer.
	$effect(() => {
		if (handoff.offered && !queue && !handoff.busy) handoff.dismiss();
	});

	async function go(id = target) {
		const c = choices.find((x) => x.id === id);
		if (!c || handoff.busy || !queue) return;
		picked = c.id;
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

	function choose(id: string) {
		choosing = false;
		void go(id);
	}

	const count = $derived(queue?.tracks.length ?? 0);
	const offered = $derived(handoff.offered && !!queue && choices.length > 0);
	const text = $derived.by(() => {
		if (!offered) return '';
		if (handoff.busy) return `Moving to ${handoff.busy}…`;
		if (message) return message;
		return `Back online. Continue ${count} ${count === 1 ? 'track' : 'tracks'} from your downloads on ${targetName}?`;
	});
</script>

<div class="continue-on" data-continue-on={offered ? 'offered' : undefined}>
	<Snackbar
		message={text}
		action={offered && !handoff.busy ? 'Continue' : ''}
		onaction={() => void go()}
		secondary={offered && !handoff.busy && choices.length > 1 ? 'Other player…' : ''}
		onsecondary={() => (choosing = true)}
		ondismiss={() => !handoff.busy && handoff.dismiss()}
		timeout={0}
	/>
</div>
{#if choosing && offered}
	<Sheet title="Continue on" onclose={() => (choosing = false)}>
		<ul class="choices" data-continue-choices>
			{#each choices as c (c.id)}
				<li>
					<button class="row" aria-current={c.id === target ? 'true' : undefined} onclick={() => choose(c.id)}>
						<Icon name={c.device ? 'device' : 'speaker'} /><span>{c.name}</span>
					</button>
				</li>
			{/each}
		</ul>
	</Sheet>
{/if}

<style>
	.choices {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 2px;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		min-height: 52px;
		padding: 0.5rem 0.75rem;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: var(--fg);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.row[aria-current='true'] {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
</style>
