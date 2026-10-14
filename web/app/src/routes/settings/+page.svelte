<script lang="ts">
	// Settings (Wave 3.5, rule 5): an index of a handful of rows, each with a one-line summary of
	// its current value, opening a page of its own (routes/settings/[section]). Nothing was
	// removed: every setting the single long page had is on one of these pages.
	import { app, session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import { appearance } from '$lib/theme/appearance.svelte';
	import { THEMES } from '$lib/theme/themes';
	import { install } from '$lib/pwa/install.svelte';
	import { lyrics } from '$lib/state/lyrics';
	import { nowPlaying } from '$lib/nowplaying/store.svelte';
	import { STYLE_LABELS } from '$lib/visual/draw';
	import { ratings } from '$lib/state/ratings';
	import { downloads } from '$lib/offline/index.svelte';
	import { formatBytes, loadPrefs } from '$lib/offline/settings';
	import { loadDefault } from '$lib/player/default-player';
	import { textOf } from '$lib/lms/format';
	import { SECTIONS, type SectionId } from '$lib/settings/sections';
	import Icon from '$lib/components/Icon.svelte';
	import pkg from '../../../package.json';

	const user = $derived(session.current?.user.username ?? '');
	const MODE_LABELS: Record<string, string> = { auto: 'Automatic', light: 'Light', dark: 'Dark' };
	const QUALITY_LABELS: Record<string, string> = { auto: 'Automatic', original: 'Original', high: 'High', low: 'Data saver' };

	const startsOn = $derived.by(() => {
		const d = loadDefault(user);
		if (d.kind === 'device') return 'This device';
		if (d.kind === 'last') return 'The player used last';
		return textOf(app.players.find((p) => p.playerid === d.id)?.name) || 'A player';
	});
	const dl = $derived(loadPrefs());

	const summaries = $derived<Record<SectionId, string>>({
		account: user ? `Signed in as ${user}` : 'Signed in',
		appearance: [THEMES.find((t) => t.id === appearance.prefs.theme)?.name ?? '', MODE_LABELS[appearance.prefs.mode] ?? ''].filter(Boolean).join(' · '),
		playback: `Starts on ${startsOn} · Quality: ${QUALITY_LABELS[device.qualityPreference] ?? device.qualityPreference}`,
		'now-playing': [`Lyric line ${lyrics.peek ? 'on' : 'off'}`, nowPlaying.visualiser ? `Visualiser: ${STYLE_LABELS[nowPlaying.style]}` : 'Visualiser off', ratings.style === 'heart' ? 'Rate with a heart' : 'Rate with stars'].join(' · '),
		downloads: `${downloads.saved.length} ${downloads.saved.length === 1 ? 'track' : 'tracks'}, ${formatBytes(downloads.usedBytes)}${dl.wifiOnly ? ' · Wi-Fi only' : ''}`,
		history: 'Your listening, and what is kept',
		install: 'Open it full screen like any other app',
		about: `Version ${pkg.version} · Advanced`,
		advanced: '',
	});
	/** The rows: every top-level section; Install only where the browser can. */
	const rows = $derived(SECTIONS.filter((s) => !s.parent && (s.id !== 'install' || install.way === 'prompt' || install.way === 'ios')));
</script>

<section class="page settings">
	<h1>Settings</h1>
	<nav aria-label="Settings">
		<ul class="index" data-settings-index>
			{#each rows as s (s.id)}
				<li>
					<a class="row" href={`#/settings/${s.id}`} data-settings-row={s.id}>
						<span class="ic"><Icon name={s.icon} /></span>
						<span class="grow">
							<span class="label">{s.label}</span>
							<span class="summary muted ellipsis" data-summary>{summaries[s.id]}</span>
						</span>
						<span class="chev" aria-hidden="true">›</span>
					</a>
				</li>
			{/each}
		</ul>
	</nav>
</section>

<style>
	.settings {
		max-width: 760px;
	}
	.index {
		list-style: none;
		margin: 0.5rem 0 0;
		padding: 0;
		display: grid;
		gap: 2px;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.9rem;
		min-height: 60px;
		padding: 0.5rem 0.6rem;
		border-radius: var(--radius-sm);
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.ic {
		display: grid;
		place-items: center;
		width: 40px;
		height: 40px;
		flex: none;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.grow {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.label {
		font-weight: 600;
	}
	.summary {
		font-size: 0.85rem;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.3rem;
	}
</style>
