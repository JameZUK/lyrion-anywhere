<script lang="ts">
	// The sleep timer (docs/plans: Wave 1 §1): fixed lengths, a custom length, or the end of
	// this track, album or queue; for this player, its sync group, or every player. One tap
	// turns a running timer off.
	import { app } from '$lib/state/app.svelte';
	import { END_LABELS, SLEEP_MINUTES, customMinutes, formatLeft, groupOf, sleepLeft, type SleepChoice, type SleepEnd, type SleepScope } from '$lib/lms/sleep';
	import { MAX_CUSTOM_MINUTES, MIN_CUSTOM_MINUTES } from '$lib/lms/sleep';
	import { isRemote } from '$lib/lms/format';
	import Sheet from './Sheet.svelte';

	let { onclose }: { onclose: () => void } = $props();

	let scope = $state<SleepScope>('player');
	let custom = $state('');
	let busy = $state(false);
	let error = $state('');

	const left = $derived(sleepLeft(app.now));
	const grouped = $derived(groupOf(app.playerId, app.now).length > 1);
	const others = $derived(app.players.filter((p) => p.playerid !== app.playerId && p.connected !== 0).length);
	const track = $derived(app.now?.playlist_loop?.[0]);
	/** "End of …" needs a track with a length (not a radio stream). */
	const ends = $derived(!!track && !isRemote(track.remote) && Number(app.now?.duration ?? track.duration ?? 0) > 0);
	const fade = $derived(app.sleepFade);

	async function set(choice: SleepChoice) {
		busy = true;
		error = await app.setSleep(choice, scope);
		busy = false;
		if (!error) onclose();
	}

	async function off() {
		busy = true;
		error = await app.cancelSleep();
		busy = false;
	}

	function setCustom(e: SubmitEvent) {
		e.preventDefault();
		const m = customMinutes(custom);
		if (m === null) {
			error = `Enter a whole number of minutes, ${MIN_CUSTOM_MINUTES} to ${MAX_CUSTOM_MINUTES}.`;
			return;
		}
		void set({ kind: 'minutes', minutes: m });
	}

	const END_KINDS: SleepEnd[] = ['track', 'album', 'queue'];
</script>

<Sheet title="Sleep timer" {onclose}>
	<div class="sleep" data-sleep-sheet>
		{#if left !== null}
			<div class="running" role="status">
				<span>Stops in <strong>{formatLeft(left)}</strong>{app.armed ? ` (${END_LABELS[app.armed.end].toLowerCase()})` : ''}</span>
				<button class="btn primary" onclick={off} disabled={busy} data-sleep-off>Turn off</button>
			</div>
		{/if}

		{#if grouped || others > 0}
			<fieldset class="scope">
				<legend>Stop</legend>
				<label><input type="radio" bind:group={scope} value="player" /> This player</label>
				{#if grouped}<label><input type="radio" bind:group={scope} value="group" /> This sync group</label>{/if}
				{#if others > 0}<label><input type="radio" bind:group={scope} value="all" /> All players</label>{/if}
			</fieldset>
		{/if}

		<div class="grid" role="group" aria-label="Stop after">
			{#each SLEEP_MINUTES as m (m)}
				<button class="btn" onclick={() => set({ kind: 'minutes', minutes: m })} disabled={busy}>{m} min</button>
			{/each}
		</div>
		<form class="custom" onsubmit={setCustom}>
			<label for="sleep-custom">Custom</label>
			<input id="sleep-custom" type="number" inputmode="numeric" min={MIN_CUSTOM_MINUTES} max={MAX_CUSTOM_MINUTES} step="1" placeholder="Minutes" bind:value={custom} />
			<button class="btn" type="submit" disabled={busy}>Set</button>
		</form>

		<div class="ends" role="group" aria-label="Stop at the end of">
			{#each END_KINDS as k (k)}
				<button class="btn" onclick={() => set({ kind: k })} disabled={busy || !ends}>{END_LABELS[k]}</button>
			{/each}
			{#if !ends}<p class="muted small">Not for a stream without a set length.</p>{/if}
		</div>

		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<p class="muted small">
			{#if fade > 0}Fades out over {fade} s on this device (Settings).{:else}No fade-out on this device (Settings).{/if}
			Other players fade over LMS's last minute, then stop and turn off.
		</p>
	</div>
</Sheet>

<style>
	.sleep {
		display: grid;
		gap: 0.9rem;
	}
	.running {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
		padding: 0.6rem 0.75rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.scope {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem 1rem;
		border: 0;
		margin: 0;
		padding: 0;
	}
	.scope legend {
		font-weight: 600;
		margin-bottom: 0.3rem;
		padding: 0;
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(5.5rem, 1fr));
		gap: 0.5rem;
	}
	.custom {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	.custom input {
		width: 7rem;
		padding: 0.5rem 0.6rem;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg);
	}
	.ends {
		display: grid;
		gap: 0.5rem;
	}
	.ends .btn {
		justify-content: flex-start;
	}
	.small {
		font-size: 0.85rem;
		margin: 0;
	}
	.error {
		color: var(--danger);
		margin: 0;
	}
</style>
