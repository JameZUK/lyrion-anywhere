<script lang="ts">
	// Keyboard shortcuts for the whole app (see lib/keys/shortcuts.ts), and their help sheet.
	// What a shortcut did is announced politely for screen readers.
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { tick } from 'svelte';
	import { app } from '#lib/state/app.svelte';
	import * as cmd from '#lib/lms/commands';
	import { device } from '#lib/player/device.svelte';
	import { controls } from '#lib/player/controls.svelte';
	import { nowPlaying } from '#lib/nowplaying/store.svelte';
	import { haptics } from '#lib/haptics/haptics.svelte';
	import { levelFixed } from '#lib/player/volume';
	import { SEEK_STEP, SHORTCUT_HELP, VOLUME_STEP, shortcutFor, type Shortcut } from '#lib/keys/shortcuts';
	import { keysHelp } from '#lib/keys/help.svelte';
	import Sheet from './Sheet.svelte';

	/** Where the app is (read at each key press). */
	const here = () => (page.url?.hash ?? '').replace(/^#/, '').replace(/[?#].*$/, '') || '/';
	/** Pages that don't scroll, where the arrow keys are free. */
	const arrowsFree = (path: string) => path === '/' || path === '/car';
	let said = $state('');

	function say(text: string) {
		said = '';
		void tick().then(() => (said = text));
	}

	async function run(s: Shortcut, path: string) {
		const now = app.now;
		const duration = Number(now?.duration ?? 0);
		// What works now (out of reach, only what this device can do by itself).
		const g = controls.gates;
		switch (s) {
			case 'playpause': {
				if (!now || !g.playPause) return;
				const was = app.playing;
				controls.toggle();
				say(was ? 'Paused' : 'Playing');
				return;
			}
			case 'seekback':
			case 'seekforward': {
				if (!now || !(duration > 0) || now.can_seek === 0 || !g.seek) return;
				const to = Math.max(0, Math.min(duration - 1, app.elapsed + (s === 'seekback' ? -SEEK_STEP : SEEK_STEP)));
				await controls.seek(to);
				return;
			}
			case 'previous':
			case 'next':
				if (!now || !(s === 'next' ? g.next : g.previous)) return;
				haptics.tick('skip');
				if (s === 'next') controls.next();
				else controls.previous();
				say(s === 'next' ? 'Next track' : 'Previous track');
				return;
			case 'volumeup':
			case 'volumedown': {
				if (!now || !g.volume || levelFixed(now) || device.buttonsOnly(app.playerId)) return;
				const cur = Number(now['mixer volume'] ?? 0);
				const v = Math.max(0, Math.min(100, Math.abs(cur) + (s === 'volumeup' ? VOLUME_STEP : -VOLUME_STEP)));
				haptics.volume(Math.abs(cur), v);
				await app.send(cmd.volume(v));
				say(`Volume ${v}`);
				return;
			}
			case 'search':
				if (path !== '/search') await goto('#/search');
				await tick();
				document.querySelector<HTMLInputElement>('[data-search-input]')?.focus();
				return;
			case 'lyrics': {
				const showing = path === '/' && nowPlaying.layout === 'lyrics';
				nowPlaying.setLayout(showing ? 'art' : 'lyrics');
				if (path !== '/') await goto('#/');
				say(showing ? 'Art' : 'Lyrics');
				return;
			}
			case 'help':
				keysHelp.open = true;
				return;
		}
	}

	function onkeydown(e: KeyboardEvent) {
		const path = here();
		const s = shortcutFor(e, { arrows: arrowsFree(path) });
		if (!s) return;
		e.preventDefault();
		void run(s, path);
	}
</script>

<svelte:window {onkeydown} />

<div class="sr-only" aria-live="polite" data-shortcut-said>{said}</div>

{#if keysHelp.open}
	<Sheet title="Keyboard shortcuts" onclose={() => (keysHelp.open = false)}>
		<p class="muted small">They don’t act while you type, or in a dialog or menu.</p>
		<dl class="keys" data-shortcut-help>
			{#each SHORTCUT_HELP as h (h.label)}
				<div class="row">
					<dt>
						{#each h.keys as k, i (k)}{#if i > 0}<span class="sep" aria-hidden="true">{h.keys[0] === 'Shift' && i === 1 ? '+' : '/'}</span>{/if}<kbd>{k}</kbd>{/each}
					</dt>
					<dd>{h.label}{#if h.where}<span class="muted where"> ({h.where})</span>{/if}</dd>
				</div>
			{/each}
		</dl>
	</Sheet>
{/if}

<style>
	.keys {
		margin: 0;
		display: grid;
		gap: 0.35rem;
	}
	.row {
		display: grid;
		grid-template-columns: 7.5rem 1fr;
		gap: 0.75rem;
		align-items: baseline;
	}
	dt {
		white-space: nowrap;
	}
	dd {
		margin: 0;
	}
	kbd {
		display: inline-block;
		min-width: 1.6em;
		padding: 0.1em 0.4em;
		border: 1px solid var(--line);
		border-bottom-width: 2px;
		border-radius: 4px;
		background: var(--bg-sunken);
		font: inherit;
		font-size: 0.85rem;
		text-align: center;
	}
	.sep {
		padding: 0 0.15em;
	}
	.small,
	.where {
		font-size: 0.85rem;
	}
	.small {
		margin: 0 0 0.6rem;
	}
</style>
