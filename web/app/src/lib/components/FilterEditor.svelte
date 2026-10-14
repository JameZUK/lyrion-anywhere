<script lang="ts">
	// The smart filter rule builder (wave2-design.md §5.6): a name, up to one rule of each kind
	// (all must match), the order and how many albums. Everything is checked against the
	// contract's limits before it is sent; ids come only from the server (genres and artists
	// as LMS lists them).
	import { api } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { ROLE_LABELS } from '$lib/lms/credits';
	import type { AlbumsResult, ArtistsResult, GenresResult } from '$lib/lms/types';
	import {
		DECADES,
		FIELD_LABELS,
		FILTER_FIELDS,
		FILTER_LIMITS,
		FILTER_SORTS,
		SORT_LABELS,
		cleanFilterName,
		filterProblem,
		type FilterField,
		type FilterRule,
		type FilterSort,
		type FilterSpec,
		type SmartFilter,
	} from '$lib/api/filters';
	import Sheet from './Sheet.svelte';
	import Icon from './Icon.svelte';

	let {
		initial = null,
		onsave,
		oncancel,
	}: {
		initial?: SmartFilter | null;
		/** Save the filter: resolves to '' when saved, or why not. */
		onsave: (spec: FilterSpec) => Promise<string>;
		oncancel: () => void;
	} = $props();

	const start = (() => {
		const f = initial;
		const rule = <F extends FilterField>(field: F) => f?.rules.find((r) => r.field === field) as Extract<FilterRule, { field: F }> | undefined;
		return { f, genre: rule('genre'), decade: rule('decade'), artist: rule('artist'), compilation: rule('compilation') };
	})();

	let name = $state(start.f?.name ?? '');
	let active = $state<FilterField[]>(start.f ? start.f.rules.map((r) => r.field) : []);
	let picked = $state<number[]>(start.genre ? [...start.genre.in] : []);
	let decade = $state<number | ''>(start.decade?.is ?? '');
	let artist = $state<{ id: number; name: string } | null>(start.artist ? { id: start.artist.is, name: '' } : null);
	let role = $state<cmd.Role | ''>(start.artist?.role ?? '');
	let compilation = $state<boolean>(start.compilation?.is ?? true);
	let sort = $state<FilterSort>(start.f?.sort ?? 'album');
	let limit = $state<number>(start.f?.limit ?? 100);
	let adding = $state<FilterField | ''>('');
	let error = $state('');
	let saving = $state(false);

	let genres = $state<{ id: number; genre: string }[] | null>(null);
	let genreError = $state('');
	async function loadGenres() {
		if (genres) return;
		try {
			const r = await api.rpc<GenresResult>('', cmd.genres(0, 500));
			genres = (r.genres_loop ?? []).filter((g) => Number.isSafeInteger(g.id) && g.id > 0).map((g) => ({ id: g.id, genre: textOf(g.genre) }));
		} catch (e) {
			genreError = (e as Error).message;
		}
	}
	$effect(() => {
		if (active.includes('genre')) void loadGenres();
	});

	// The chosen artist's name, when editing a saved filter (only its id is saved).
	$effect(() => {
		const a = artist;
		if (!a || a.name) return;
		let live = true;
		api.rpc<AlbumsResult>('', cmd.artistAlbums(a.id)).then(
			(r) => {
				const names = (r.albums_loop ?? []).map((x) => textOf(x.artist)).filter(Boolean);
				if (live && artist?.id === a.id) artist = { id: a.id, name: names[0] ?? 'Chosen artist' };
			},
			() => live && artist?.id === a.id && (artist = { id: a.id, name: 'Chosen artist' }),
		);
		return () => (live = false);
	});

	let term = $state('');
	let found = $state<{ id: number; name: string }[]>([]);
	let searching = $state(false);
	$effect(() => {
		const t = term.trim();
		if (t.length < 2) {
			found = [];
			return;
		}
		let live = true;
		const timer = setTimeout(async () => {
			searching = true;
			try {
				const r = await api.rpc<ArtistsResult>('', cmd.artists(0, 10, t));
				if (live) found = (r.artists_loop ?? []).filter((a) => Number.isSafeInteger(a.id) && a.id > 0).map((a) => ({ id: a.id, name: textOf(a.artist) }));
			} catch {
				if (live) found = [];
			} finally {
				if (live) searching = false;
			}
		}, 250);
		return () => {
			live = false;
			clearTimeout(timer);
		};
	});

	const addable = $derived(FILTER_FIELDS.filter((f) => !active.includes(f)));
	const full = $derived(active.length >= FILTER_LIMITS.maxRules);

	function addRule() {
		if (!adding || active.includes(adding) || full) return;
		active = [...active, adding];
		const added = adding;
		adding = '';
		error = '';
		queueMicrotask(() => document.querySelector<HTMLElement>(`[data-rule="${added}"] [data-rule-first]`)?.focus());
	}
	function removeRule(f: FilterField) {
		active = active.filter((x) => x !== f);
		error = '';
	}
	function toggleGenre(id: number, on: boolean) {
		picked = on ? (picked.includes(id) || picked.length >= FILTER_LIMITS.genresMax ? picked : [...picked, id]) : picked.filter((x) => x !== id);
	}

	/** The filter as built so far (checked by `filterProblem` before saving). */
	export function build(): FilterSpec {
		const rules = active.map((f): FilterRule => {
			switch (f) {
				case 'genre':
					return { field: 'genre', in: [...picked] };
				case 'decade':
					return { field: 'decade', is: decade === '' ? 0 : Number(decade) };
				case 'artist':
					return role ? { field: 'artist', is: artist?.id ?? 0, role } : { field: 'artist', is: artist?.id ?? 0 };
				case 'compilation':
					return { field: 'compilation', is: compilation };
			}
		});
		return { name: cleanFilterName(name), rules, sort, limit: Number(limit) };
	}

	const sentence = (p: string) => `${p.charAt(0).toUpperCase()}${p.slice(1)}.`;

	async function save(e: SubmitEvent) {
		e.preventDefault();
		const spec = build();
		const p = filterProblem(spec);
		if (p) {
			error = sentence(p);
			return;
		}
		saving = true;
		error = '';
		try {
			error = await onsave(spec);
		} finally {
			saving = false;
		}
	}
</script>

<Sheet title={initial ? 'Edit smart filter' : 'New smart filter'} onclose={oncancel}>
	<form class="editor" onsubmit={save} novalidate data-filter-editor>
		<label class="field">
			<span>Name</span>
			<input type="text" bind:value={name} maxlength={FILTER_LIMITS.nameMax} autocomplete="off" required data-filter-name />
		</label>

		<fieldset class="rules">
			<legend>Rules <span class="muted small">(albums must match all of them)</span></legend>
			{#if !active.length}<p class="muted small">Add at least one rule.</p>{/if}
			{#each active as f (f)}
				<div class="rule" data-rule={f} role="group" aria-labelledby={`rule-${f}`}>
					<div class="rule-head">
						<h3 id={`rule-${f}`}>{FIELD_LABELS[f]}</h3>
						<button type="button" class="icon-btn" aria-label={`Remove the ${FIELD_LABELS[f].toLowerCase()} rule`} onclick={() => removeRule(f)} data-rule-remove={f}><Icon name="minus" size={16} /></button>
					</div>
					{#if f === 'genre'}
						<p class="muted small" id="genre-count" aria-live="polite">{picked.length} of up to {FILTER_LIMITS.genresMax} chosen</p>
						{#if genreError}<p class="error">{genreError}</p>
						{:else if !genres}<p class="muted small">Loading genres…</p>
						{:else if !genres.length}<p class="muted small">No genres in the library.</p>
						{:else}
							<ul class="genres" aria-describedby="genre-count">
								{#each genres as g, i (g.id)}
									<li>
										<label class="check">
											<input
												type="checkbox"
												checked={picked.includes(g.id)}
												disabled={!picked.includes(g.id) && picked.length >= FILTER_LIMITS.genresMax}
												onchange={(e) => toggleGenre(g.id, e.currentTarget.checked)}
												data-rule-first={i === 0 ? '' : undefined}
												data-genre={g.id}
											/>
											<span>{g.genre}</span>
										</label>
									</li>
								{/each}
							</ul>
						{/if}
					{:else if f === 'decade'}
						<label class="field">
							<span>Decade is</span>
							<select bind:value={decade} data-rule-first data-decade>
								<option value="">Choose…</option>
								{#each DECADES as d (d)}<option value={d}>{d}s</option>{/each}
							</select>
						</label>
					{:else if f === 'artist'}
						{#if artist}
							<p class="chosen">
								<span>Artist is <strong>{artist.name || '…'}</strong></span>
								<button type="button" class="btn small" onclick={() => (artist = null)} data-artist-change>Change</button>
							</p>
						{:else}
							<label class="field">
								<span>Find an artist</span>
								<input type="search" bind:value={term} autocomplete="off" data-rule-first data-artist-search />
							</label>
							<div aria-live="polite" class="small muted">{searching ? 'Searching…' : term.trim().length >= 2 && !found.length ? 'No artists found.' : ''}</div>
							{#if found.length}
								<ul class="found">
									{#each found as a (a.id)}
										<li><button type="button" class="btn" onclick={() => ((artist = a), (term = ''), (found = []))} data-artist-pick={a.id}>{a.name}</button></li>
									{/each}
								</ul>
							{/if}
						{/if}
						<label class="field">
							<span>In the role</span>
							<select bind:value={role} data-artist-role>
								<option value="">Any role</option>
								{#each cmd.ROLES as r (r)}<option value={r}>{ROLE_LABELS[r]}</option>{/each}
							</select>
						</label>
					{:else}
						<div class="choices" role="radiogroup" aria-labelledby={`rule-${f}`}>
							<label class="check"><input type="radio" name="compilation" checked={compilation} onchange={() => (compilation = true)} data-rule-first data-compilation="yes" /><span>Compilations only</span></label>
							<label class="check"><input type="radio" name="compilation" checked={!compilation} onchange={() => (compilation = false)} data-compilation="no" /><span>No compilations</span></label>
						</div>
					{/if}
				</div>
			{/each}
			{#if addable.length && !full}
				<div class="add">
					<label class="field">
						<span>Add a rule</span>
						<select bind:value={adding} data-rule-add>
							<option value="">Choose…</option>
							{#each addable as f (f)}<option value={f}>{FIELD_LABELS[f]}</option>{/each}
						</select>
					</label>
					<button type="button" class="btn" onclick={addRule} disabled={!adding} data-rule-add-button><Icon name="plus" size={18} />Add</button>
				</div>
			{/if}
		</fieldset>

		<div class="row">
			<label class="field">
				<span>Order</span>
				<select bind:value={sort} data-filter-sort>
					{#each FILTER_SORTS as s (s)}<option value={s}>{SORT_LABELS[s]}</option>{/each}
				</select>
			</label>
			<label class="field">
				<span>Up to (albums)</span>
				<input type="number" inputmode="numeric" min={FILTER_LIMITS.limitMin} max={FILTER_LIMITS.limitMax} step="1" bind:value={limit} data-filter-limit />
			</label>
		</div>

		<p class="error" role="alert" data-filter-error>{error}</p>
		<div class="actions">
			<button type="button" class="btn" onclick={oncancel}>Cancel</button>
			<button type="submit" class="btn primary" disabled={saving} data-filter-save>Save</button>
		</div>
	</form>
</Sheet>

<style>
	.editor {
		display: grid;
		gap: 0.9rem;
	}
	.rules {
		border: 0;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.75rem;
	}
	legend {
		font-weight: 650;
		margin-bottom: 0.4rem;
	}
	.rule {
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem 0.75rem;
		display: grid;
		gap: 0.5rem;
	}
	.rule-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	h3 {
		margin: 0;
		font-size: 1rem;
	}
	.genres {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 14rem;
		overflow-y: auto;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
	}
	.check {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		min-height: 44px;
	}
	.check input {
		width: 20px;
		height: 20px;
		accent-color: var(--accent);
	}
	.found {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}
	.found .btn,
	.chosen .btn {
		min-height: 44px;
	}
	.chosen {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		margin: 0;
	}
	.add,
	.row {
		display: flex;
		flex-wrap: wrap;
		align-items: end;
		gap: 0.5rem 0.75rem;
	}
	.add .btn {
		min-height: 44px;
	}
	.row .field {
		flex: 1 1 10rem;
	}
	.field select,
	.field input {
		min-height: 44px;
		box-sizing: border-box;
	}
	.error:empty {
		display: none;
	}
	.error {
		margin: 0;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
	}
	.actions .btn {
		min-height: 44px;
	}
	.small {
		font-size: 0.85rem;
	}
	p {
		margin: 0;
	}
</style>
