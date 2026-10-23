<script lang="ts">
	// Settings › About (Wave 3.5): the version, keyboard shortcuts, the project, and Advanced.
	import { version as buildVersion } from '$app/env';
	import { keysHelp } from '#lib/keys/help.svelte';
	import Icon from '#lib/components/Icon.svelte';
</script>

<div class="about-row">
	<img src="./icons/icon.svg" alt="" width="64" height="64" />
	<div>
		<p class="about-name">Lyrion Anywhere</p>
		<!-- The plugin's release version (the release this web app shipped in), and the build. -->
		<p class="muted small" data-app-version>Version {__PLUGIN_VERSION__} · build {buildVersion}</p>
	</div>
</div>
<p class="muted small">Your Lyrion Music Server, securely, from anywhere. Free software under the GNU GPL (version 2 or later).</p>

<div class="rows">
	<button class="row" onclick={() => (keysHelp.open = true)} aria-haspopup="dialog" data-keys-help>
		<Icon name="keyboard" /><span class="grow">Keyboard shortcuts<small class="muted">Or press ? anywhere</small></span><span class="chev" aria-hidden="true">›</span>
	</button>
	<a class="row" href="https://github.com/JameZUK/lyrion-anywhere" target="_blank" rel="noopener noreferrer">
		<Icon name="info" /><span class="grow">Project page and source code</span><span class="chev" aria-hidden="true">↗</span>
	</a>
	<a class="row" href="#/settings/advanced" data-advanced-link>
		<Icon name="settings" /><span class="grow">Advanced<small class="muted">This device’s player, playback engine, volume source, diagnostics</small></span><span class="chev" aria-hidden="true">›</span>
	</a>
</div>

<style>
	.about-row {
		display: flex;
		align-items: center;
		gap: 1rem;
	}
	.about-row img {
		flex: none;
	}
	.about-name {
		margin: 0;
		font-weight: 700;
		font-size: 1.1rem;
	}
	.small {
		font-size: 0.85rem;
		margin: 0.15rem 0 0;
	}
	.rows {
		display: grid;
		gap: 2px;
		margin-top: 1rem;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		min-height: 52px;
		padding: 0.5rem;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.row :global(svg) {
		color: var(--fg-muted);
		flex: none;
	}
	.grow {
		flex: 1;
		display: grid;
		font-weight: 550;
	}
	.grow small {
		font-weight: 400;
		font-size: 0.82rem;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.2rem;
	}
</style>
