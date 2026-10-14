<script lang="ts">
	// One page of Settings (Wave 3.5): Account, Appearance, Playback, Now playing, Downloads,
	// Listening history, Install the app, About and (under About) Advanced. Back returns to the
	// index (or to About, from Advanced). An address that names no section says so.
	import { page } from '$app/state';
	import { useDetailTitle } from '$lib/title.svelte';
	import { install } from '$lib/pwa/install.svelte';
	import { sectionOf } from '$lib/settings/sections';
	import BackButton from '$lib/components/BackButton.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import AppearancePicker from '$lib/components/AppearancePicker.svelte';
	import HistorySettings from '$lib/components/HistorySettings.svelte';
	import AccountSettings from '$lib/settings/AccountSettings.svelte';
	import PlaybackSettings from '$lib/settings/PlaybackSettings.svelte';
	import NowPlayingSettings from '$lib/settings/NowPlayingSettings.svelte';
	import DownloadSettings from '$lib/settings/DownloadSettings.svelte';
	import AboutSettings from '$lib/settings/AboutSettings.svelte';
	import AdvancedSettings from '$lib/settings/AdvancedSettings.svelte';

	const section = $derived(sectionOf(page.params.section));
	useDetailTitle(() => section?.label ?? '');
</script>

<section class="page settings" data-settings-section={section?.id}>
	<BackButton parent={section?.parent ? `/settings/${section.parent}` : '/settings'} />
	{#if !section}
		<p class="error">Not a setting.</p>
		<p><a class="link" href="#/settings">All settings</a></p>
	{:else}
		<h1>{section.label}</h1>
		{#if section.id === 'account'}
			<AccountSettings />
		{:else if section.id === 'appearance'}
			<AppearancePicker />
		{:else if section.id === 'playback'}
			<PlaybackSettings />
		{:else if section.id === 'now-playing'}
			<NowPlayingSettings />
		{:else if section.id === 'downloads'}
			<DownloadSettings />
		{:else if section.id === 'history'}
			<HistorySettings />
			<a class="row" href="#/stats" data-stats-link><Icon name="calendar" /><span class="grow">Your listening</span><span class="chev" aria-hidden="true">›</span></a>
		{:else if section.id === 'install'}
			{#if install.way === 'prompt'}
				<p>Add Lyrion Anywhere to your home screen: it opens full screen like any other app, and remembers your settings.</p>
				<button class="btn primary" onclick={() => void install.install()}><Icon name="plus" size={18} />Install</button>
			{:else if install.way === 'ios'}
				<p>Add Lyrion Anywhere to your home screen: it opens full screen like any other app.</p>
				<ol class="steps">
					<li>Tap the <strong>Share</strong> button (the square with an arrow).</li>
					<li>Choose <strong>Add to Home Screen</strong>.</li>
					<li>Tap <strong>Add</strong>.</li>
				</ol>
			{:else}
				<p class="muted">This browser has installed the app already, or can’t install it.</p>
			{/if}
		{:else if section.id === 'about'}
			<AboutSettings />
		{:else}
			<AdvancedSettings />
		{/if}
	{/if}
</section>

<style>
	.settings {
		max-width: 760px;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 52px;
		margin-top: 1rem;
		padding: 0.5rem;
		border-radius: var(--radius-sm);
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.row :global(svg) {
		color: var(--fg-muted);
	}
	.grow {
		flex: 1;
		font-weight: 550;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.2rem;
	}
	.steps {
		margin: 0;
		padding-left: 1.3rem;
		display: grid;
		gap: 0.3rem;
	}
	.link {
		text-decoration: underline;
	}
</style>
