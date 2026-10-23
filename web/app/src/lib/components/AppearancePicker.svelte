<script lang="ts">
	// Settings › Appearance: theme (with a live preview of each), light/dark, accent colour.
	import { appearance } from '#lib/theme/appearance.svelte';
	import { ACCENTS, THEMES, tokens, type Mode } from '#lib/theme/themes';

	const MODES: { id: Mode; label: string }[] = [
		{ id: 'auto', label: 'Automatic' },
		{ id: 'light', label: 'Light' },
		{ id: 'dark', label: 'Dark' },
	];
	const current = $derived(THEMES.find((t) => t.id === appearance.prefs.theme)!);
	const preview = (id: (typeof THEMES)[number]['id']) => tokens({ ...appearance.prefs, theme: id }, appearance.mode === 'dark');
	const isPreset = $derived(ACCENTS.some((a) => a.hex === appearance.prefs.accent));
</script>

<div class="picker">
	<div class="themes" role="radiogroup" aria-label="Theme">
		{#each THEMES as t (t.id)}
			{@const k = preview(t.id)}
			<label class="theme" class:on={appearance.prefs.theme === t.id}>
				<input type="radio" name="theme" class="sr-only" checked={appearance.prefs.theme === t.id} onchange={() => appearance.set({ theme: t.id })} />
				<span class="mock" aria-hidden="true" style:--m-bg={k.bg} style:--m-raised={k['bg-raised']} style:--m-line={k.line} style:--m-accent={k.accent} style:--m-soft={k['accent-soft']} style:--m-fg={k.fg} style:--m-radius={k['radius-sm']}>
					<span class="m-side"><span class="m-pill"></span><span class="m-line"></span><span class="m-line"></span></span>
					<span class="m-main"><span class="m-card"><span class="m-line wide"></span><span class="m-line"></span></span><span class="m-btn"></span></span>
				</span>
				<span class="name">{t.name}</span>
				<span class="desc muted">{t.description}</span>
			</label>
		{/each}
	</div>

	<div class="row" role="radiogroup" aria-label="Light or dark">
		{#each MODES as m (m.id)}
			<label class="seg" class:on={appearance.prefs.mode === m.id}>
				<input type="radio" name="mode" class="sr-only" checked={appearance.prefs.mode === m.id} onchange={() => appearance.set({ mode: m.id })} />{m.label}
			</label>
		{/each}
	</div>
	{#if appearance.prefs.mode === 'auto'}<p class="muted small">Follows this device’s light or dark setting.</p>{/if}

	{#if current.accent}
		<div class="accents" role="radiogroup" aria-label="Accent colour">
			{#each ACCENTS as a (a.hex)}
				<label class="swatch" class:on={appearance.prefs.accent === a.hex} title={a.name} style:--sw={a.hex}>
					<input type="radio" name="accent" class="sr-only" checked={appearance.prefs.accent === a.hex} onchange={() => appearance.set({ accent: a.hex })} />
					<span class="sr-only">{a.name}</span>
				</label>
			{/each}
			<label class="swatch custom" class:on={!isPreset} title="Any colour" style:--sw={isPreset ? undefined : appearance.prefs.accent}>
				<input type="color" class="sr-only" value={appearance.prefs.accent} oninput={(e) => appearance.set({ accent: (e.currentTarget as HTMLInputElement).value })} />
				<span class="sr-only">Any colour</span>
			</label>
		</div>
		<p class="muted small">Colours are built around the accent, and always stay readable.</p>
	{/if}
</div>

<style>
	.picker {
		display: grid;
		gap: 1rem;
	}
	.themes {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 190px), 1fr));
		gap: 0.75rem;
	}
	.theme {
		display: grid;
		gap: 0.3rem;
		padding: 0.6rem;
		border-radius: var(--radius);
		border: 2px solid var(--line);
		cursor: pointer;
	}
	.theme.on {
		border-color: var(--accent);
	}
	.theme:focus-within,
	.seg:focus-within,
	.swatch:focus-within {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}
	.name {
		font-weight: 650;
	}
	.desc {
		font-size: 0.82rem;
	}
	.mock {
		display: grid;
		grid-template-columns: 30% 1fr;
		height: 74px;
		border-radius: var(--m-radius);
		overflow: hidden;
		background: var(--m-bg);
		border: 1px solid var(--m-line);
	}
	.m-side {
		display: grid;
		align-content: start;
		gap: 5px;
		padding: 8px 6px;
		background: var(--m-raised);
	}
	.m-pill {
		height: 9px;
		border-radius: 99px;
		background: var(--m-soft);
	}
	.m-line {
		display: block;
		height: 5px;
		width: 70%;
		border-radius: 3px;
		background: var(--m-fg);
		opacity: 0.35;
	}
	.m-line.wide {
		width: 90%;
		opacity: 0.6;
	}
	.m-main {
		display: grid;
		align-content: space-between;
		padding: 8px;
	}
	.m-card {
		display: grid;
		gap: 5px;
		padding: 7px;
		border-radius: var(--m-radius);
		background: var(--m-raised);
		border: 1px solid var(--m-line);
	}
	.m-btn {
		justify-self: end;
		width: 34px;
		height: 12px;
		border-radius: 99px;
		background: var(--m-accent);
	}
	.row {
		display: inline-flex;
		border: 1px solid var(--line);
		border-radius: 999px;
		overflow: hidden;
		width: fit-content;
	}
	.seg {
		padding: 0.45em 1em;
		cursor: pointer;
		font-weight: 550;
	}
	.seg + .seg {
		border-left: 1px solid var(--line);
	}
	.seg.on {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.accents {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
	.swatch {
		width: 36px;
		height: 36px;
		border-radius: 50%;
		background: var(--sw);
		cursor: pointer;
		box-shadow: inset 0 0 0 1px rgb(0 0 0 / 15%);
	}
	.swatch.on {
		box-shadow:
			0 0 0 3px var(--bg-raised),
			0 0 0 5px var(--fg);
	}
	.custom {
		background: var(--sw, conic-gradient(#e53935, #fdd835, #43a047, #1e88e5, #8e24aa, #e53935));
	}
	.small {
		font-size: 0.85rem;
		margin: 0;
	}
</style>
