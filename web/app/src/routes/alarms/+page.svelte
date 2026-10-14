<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	// Alarms for the selected player (Wave 2 §1): LMS's own, so they ring on hardware
	// players too and with no app open.
	import { app } from '$lib/state/app.svelte';
	import { routine } from '$lib/state/me';
	import { alarmChanges } from '$lib/rooms/alarm-changes.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Switch from '$lib/components/Switch.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import {
		DAY_NAMES,
		daysText,
		nextAlarm,
		parseTime,
		timeText,
		type Alarm,
		type AlarmChange,
		type AlarmListing,
	} from '$lib/api/routine';

	let listing = $state<AlarmListing | null>(null);
	let error = $state('');
	let busy = $state(false);
	/** The alarm being edited ('new' for a new one). */
	let editing = $state<string | null>(null);
	let confirmDelete = $state<string | null>(null);

	// The form.
	let fTime = $state('07:00');
	let fDays = $state<number[]>([1, 2, 3, 4, 5]);
	let fRepeat = $state(true);
	let fVolume = $state(50);
	let fSound = $state('current');
	let fShuffle = $state<0 | 1 | 2>(0);

	const player = $derived(app.playerId);
	const playerName = $derived(app.current?.name ?? 'this player');
	const next = $derived(listing && listing.enabled ? nextAlarm(listing.alarms, new Date()) : null);
	const categories: Record<string, string> = {
		current: 'Current queue',
		favourites: 'Favourites',
		playlists: 'Playlists',
		random: 'Random mixes',
		sounds: 'Sounds',
		other: 'Other',
	};
	const groups = $derived.by(() => {
		const g = new Map<string, { id: string; title: string }[]>();
		for (const s of listing?.sounds ?? []) {
			const k = categories[s.category] ? s.category : 'other';
			g.set(k, [...(g.get(k) ?? []), { id: s.id, title: s.title || 'Untitled' }]);
		}
		return [...g.entries()];
	});

	/** Bumped by every request: only the latest one's answer is shown. */
	let request = 0;

	/**
	 * Ask `p`'s alarms something; show the answer only if it is still the latest request and
	 * `p` is still the selected player (the user may switch players while it is on its way).
	 */
	async function run(p: string, fn: (p: string) => Promise<AlarmListing>, change = true) {
		const mine = ++request;
		const current = () => mine === request && p === app.playerId;
		busy = true;
		error = '';
		try {
			const l = await fn(p);
			// Rooms shows each player's next alarm: it reads them again after a change.
			if (change) alarmChanges.bump();
			if (!current()) return false;
			listing = l;
			return true;
		} catch (e) {
			if (current()) error = (e as Error).message;
			return false;
		} finally {
			if (mine === request) busy = false;
		}
	}

	$effect(() => {
		const p = player;
		listing = null;
		editing = null;
		confirmDelete = null;
		error = '';
		if (p) void run(p, (q) => routine.alarms(q), false);
		else request++; // nothing on its way is wanted any more
	});

	function edit(a: Alarm | null) {
		confirmDelete = null;
		editing = a ? a.id : 'new';
		fTime = timeText(a?.time ?? 7 * 3600);
		fDays = a?.days.length ? [...a.days] : [1, 2, 3, 4, 5];
		fRepeat = a?.repeat ?? true;
		fVolume = a?.volume ?? 50;
		fSound = a?.sound?.id ?? 'current';
		fShuffle = a?.shuffle ?? 0;
	}

	function toggleDay(d: number) {
		fDays = fDays.includes(d) ? fDays.filter((x) => x !== d) : [...fDays, d];
	}

	async function save() {
		const time = parseTime(fTime);
		if (time === null) {
			error = 'Choose a time.';
			return;
		}
		if (!fDays.length) {
			error = 'Choose at least one day.';
			return;
		}
		const p = player;
		if (!p) return;
		const change: AlarmChange = { time, days: fDays, repeat: fRepeat, volume: fVolume, shuffle: fShuffle, sound: fSound };
		const id = editing;
		const ok = await run(p, (q) => (id === 'new' ? routine.addAlarm(q, { ...change, enabled: true }) : routine.changeAlarm(q, id!, change)));
		if (ok) editing = null;
	}
</script>

<section class="page alarms">
	<BackButton parent="/" />
	<header>
		<h1>Alarms</h1>
		{#if player && listing && editing === null}
			<button class="btn" onclick={() => edit(null)} data-add-alarm><Icon name="plus" size={18} />New alarm</button>
		{/if}
	</header>
	{#if !player}
		<p class="muted">No player chosen. Choose a player with Play on.</p>
	{:else}
		<p class="muted lead">On {playerName}. Alarms are kept by your music server, so they ring even with this app closed.</p>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		{#if !listing}
			<p class="muted">{busy ? 'Loading…' : ''}</p>
		{:else}
			<Switch checked={listing.enabled} disabled={busy} onchange={(v) => run(player, (q) => routine.setAllAlarms(q, v))} label="Alarms on this player" data-all-alarms />
			{#if next}
				<p class="next" data-next-alarm>Next: {next.toLocaleDateString(undefined, { weekday: 'long' })} at {timeText(next.getHours() * 3600 + next.getMinutes() * 60)}</p>
			{/if}

			{#if editing !== null}
				<form class="card editor" onsubmit={(e) => (e.preventDefault(), void save())} data-alarm-form>
					<h2>{editing === 'new' ? 'New alarm' : 'Change alarm'}</h2>
					<label class="field">
						<span>Time</span>
						<input type="time" bind:value={fTime} required data-alarm-time />
					</label>
					<fieldset class="days">
						<legend>Days</legend>
						{#each [1, 2, 3, 4, 5, 6, 0] as d (d)}
							<button type="button" class="day" class:on={fDays.includes(d)} aria-pressed={fDays.includes(d)} onclick={() => toggleDay(d)}>{DAY_NAMES[d]}</button>
						{/each}
					</fieldset>
					<Switch checked={fRepeat} onchange={(v) => (fRepeat = v)} label="Every week" note="Otherwise once." />
					<label class="field">
						<span>Play</span>
						<select bind:value={fSound} data-alarm-sound>
							{#each groups as [cat, items] (cat)}
								<optgroup label={categories[cat]}>
									{#each items as s (s.id)}<option value={s.id}>{s.title}</option>{/each}
								</optgroup>
							{/each}
						</select>
					</label>
					<label class="field">
						<span>Shuffle</span>
						<select bind:value={fShuffle}>
							<option value={0}>Off</option>
							<option value={1}>Songs</option>
							<option value={2}>Albums</option>
						</select>
					</label>
					<label class="field">
						<span>Volume {fVolume}</span>
						<input type="range" min="0" max="100" bind:value={fVolume} />
					</label>
					<div class="actions">
						<button class="btn primary" type="submit" disabled={busy}>Save</button>
						<button class="btn" type="button" onclick={() => (editing = null)}>Cancel</button>
						{#if editing !== 'new'}
							<!-- Destructive: only here, and asked first (Wave 3.5). -->
							<button class="btn danger delete" type="button" onclick={() => (confirmDelete = editing)} data-alarm-delete><Icon name="trash" size={18} />Delete alarm</button>
						{/if}
					</div>
				</form>
			{/if}

			{#if listing.alarms.length === 0 && editing === null}
				<p class="muted">No alarms yet.</p>
			{:else}
				<ul class="list">
					{#each listing.alarms as a (a.id)}
						<li class:off={!a.enabled || !listing.enabled} data-alarm={a.id}>
							<button class="main" onclick={() => edit(a)} aria-label={`Change the ${timeText(a.time)} alarm`}>
								<span class="time">{timeText(a.time)}</span>
								<span class="muted">{daysText(a.days)}{a.repeat ? '' : ' · once'} · {a.sound?.title || 'Current queue'}</span>
							</button>
							<!-- One trailing control: on or off (Delete is in the alarm's own form). -->
							<Switch compact checked={a.enabled} disabled={busy} onchange={(v) => run(player, (q) => routine.changeAlarm(q, a.id, { enabled: v }))} label={`${timeText(a.time)} alarm on`} />
						</li>
					{/each}
				</ul>
			{/if}
		{/if}
	{/if}
</section>
{#if confirmDelete && player}
	{@const id = confirmDelete}
	{@const p = player}
	<ConfirmDialog
		title="Delete this alarm?"
		confirmLabel="Delete"
		danger
		onconfirm={() => run(p, (q) => routine.deleteAlarm(q, id)).then((ok) => {
			confirmDelete = null;
			if (ok) editing = null;
		})}
		oncancel={() => (confirmDelete = null)}
	/>
{/if}

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
	}
	.delete {
		margin-left: auto;
		color: var(--danger);
	}
	.next {
		font-weight: 600;
	}
	.editor {
		margin: 1rem 0;
		display: grid;
		gap: 0.6rem;
		max-width: 28rem;
	}
	.days {
		border: 0;
		padding: 0;
		margin: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem;
	}
	.days legend {
		margin-bottom: 0.3rem;
	}
	.day {
		min-width: 2.8rem;
		padding: 0.35rem 0.5rem;
		border-radius: 999px;
		border: 1px solid var(--line);
		background: none;
		color: var(--fg);
		font: inherit;
		cursor: pointer;
	}
	.day.on {
		background: var(--accent);
		border-color: var(--accent);
		color: var(--accent-fg);
	}
	.actions {
		display: flex;
		gap: 0.5rem;
	}
	.list {
		list-style: none;
		padding: 0;
		margin: 1rem 0;
	}
	.list li {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		padding: 0.5rem 0;
		border-bottom: 1px solid var(--line);
	}
	.list li.off .time {
		color: var(--fg-muted);
	}
	.main {
		flex: 1;
		display: grid;
		text-align: left;
		background: none;
		border: 0;
		color: inherit;
		font: inherit;
		cursor: pointer;
		padding: 0;
	}
	.time {
		font-size: 1.6rem;
		font-weight: 650;
		font-variant-numeric: tabular-nums;
	}
	.error {
		color: var(--danger);
	}
</style>
