<script lang="ts">
	// Make a station: a genre or decade mix, or one of the built-in kinds.
	import { api } from '#lib/state/app.svelte';
	import { me } from '#lib/state/me';
	import * as cmd from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import type { Station, StationSpec } from '#lib/api/me';

	let { oncreated, oncancel }: { oncreated: (s: Station) => void; oncancel?: () => void } = $props();

	type Kind = 'genre' | 'decade' | 'forgotten' | 'fresh' | 'deep_cuts';
	const KINDS: { kind: Kind; label: string }[] = [
		{ kind: 'genre', label: 'Genres' },
		{ kind: 'decade', label: 'A decade' },
		{ kind: 'forgotten', label: 'Forgotten favourites' },
		{ kind: 'fresh', label: 'Fresh' },
		{ kind: 'deep_cuts', label: 'Deep cuts' },
	];
	const DECADES = [1950, 1960, 1970, 1980, 1990, 2000, 2010, 2020];
	/** The gateway takes at most this many genres for one station. */
	const MAX_GENRES = 5;

	let kind = $state<Kind>('genre');
	let name = $state('');
	let genres = $state<{ id: number; genre: string }[]>([]);
	let picked = $state<number[]>([]);
	let decade = $state(1980);
	let mine = $state(false);
	let error = $state('');
	let saving = $state(false);

	$effect(() => {
		if (kind !== 'genre' || genres.length) return;
		api
			.rpc<{ genres_loop?: { id: number; genre?: string }[] }>('', cmd.genres(0, 300))
			.then((r) => {
				genres = (r.genres_loop ?? [])
					.filter((g) => Number.isSafeInteger(g.id) && g.id > 0)
					.map((g) => ({ id: g.id, genre: textOf(g.genre) }));
			})
			.catch((e) => (error = (e as Error).message));
	});

	function toggle(id: number) {
		picked = picked.includes(id) ? picked.filter((x) => x !== id) : picked.length < MAX_GENRES ? [...picked, id] : picked;
	}

	const suggested = $derived.by(() => {
		if (kind === 'genre') return picked.map((id) => genres.find((g) => g.id === id)?.genre ?? '').filter(Boolean).join(' & ') || 'Genre mix';
		if (kind === 'decade') return `${mine ? 'My ' : ''}${String(decade).slice(2)}s`;
		return KINDS.find((k) => k.kind === kind)!.label;
	});

	async function create(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		let spec: StationSpec;
		if (kind === 'genre') {
			if (!picked.length) {
				error = 'Pick at least one genre.';
				return;
			}
			spec = { kind: 'genre', genres: [...picked] };
		} else if (kind === 'decade') spec = { kind: 'decade', decade, mine };
		else spec = { kind };
		const title = (name.trim() || suggested).slice(0, 64);
		saving = true;
		try {
			oncreated(await me.createStation(title, spec));
		} catch (err) {
			error = (err as Error).message;
		} finally {
			saving = false;
		}
	}
</script>

<form class="new" onsubmit={create}>
	<fieldset>
		<legend>Kind of station</legend>
		{#each KINDS as k (k.kind)}
			<label class="choice"><input type="radio" name="kind" value={k.kind} bind:group={kind} />{k.label}</label>
		{/each}
	</fieldset>
	{#if kind === 'genre'}
		<fieldset class="genres">
			<legend>Genres (up to {MAX_GENRES})</legend>
			{#each genres as g (g.id)}
				<label class="choice"><input type="checkbox" checked={picked.includes(g.id)} onchange={() => toggle(g.id)} />{g.genre}</label>
			{:else}
				<p class="muted">Loading genres…</p>
			{/each}
		</fieldset>
	{:else if kind === 'decade'}
		<label class="field"
			><span>Decade</span>
			<select bind:value={decade}>
				{#each DECADES as d (d)}<option value={d}>{d}s</option>{/each}
			</select>
		</label>
		<label class="choice"><input type="checkbox" bind:checked={mine} />Mostly what I’ve played from then</label>
	{/if}
	<label class="field"><span>Name</span><input bind:value={name} placeholder={suggested} maxlength="64" /></label>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<div class="buttons">
		<button class="btn primary" type="submit" disabled={saving}>Create station</button>
		{#if oncancel}<button class="btn" type="button" onclick={oncancel}>Cancel</button>{/if}
	</div>
</form>

<style>
	.new {
		display: grid;
		gap: 0.75rem;
		max-width: 560px;
	}
	fieldset {
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem 1rem;
		padding: 0.5rem 0.75rem;
	}
	.genres {
		max-height: 14rem;
		overflow-y: auto;
	}
	.choice {
		display: inline-flex;
		align-items: center;
		gap: 0.4rem;
	}
	.buttons {
		display: flex;
		gap: 0.5rem;
	}
</style>
