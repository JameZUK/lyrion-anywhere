<script lang="ts">
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { coverPath } from '$lib/lms/commands';
	import { duration, textOf } from '$lib/lms/format';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import PlayerSwitcher from './PlayerSwitcher.svelte';

	let switcher = $state(false);
	let seeking = $state<number | null>(null);
	let volume = $state<number | null>(null);
	let volTimer: ReturnType<typeof setTimeout> | undefined;

	const track = $derived(app.now?.playlist_loop?.[0]);
	const total = $derived(app.now?.duration ?? track?.duration ?? 0);
	const shownTime = $derived(seeking ?? app.elapsed);
	const vol = $derived(volume ?? app.now?.['mixer volume'] ?? 0);
	const title = $derived(textOf(track?.title ?? app.now?.current_title) || (app.current ? 'Nothing playing' : ''));
	const sub = $derived([textOf(track?.artist), textOf(track?.album)].filter(Boolean).join(' · '));
	const canSeek = $derived(total > 0 && app.now?.can_seek !== 0);

	function setVolume(v: number) {
		volume = v;
		clearTimeout(volTimer);
		volTimer = setTimeout(async () => {
			await app.send(cmd.volume(v));
			volume = null;
		}, 150);
	}
</script>

<footer class="bar" aria-label="Player">
	<a class="now" href="#/" aria-label="Now playing">
		<div class="thumb"><Art path={coverPath(track?.coverid ?? track?.artwork_track_id, 120)} alt="" label={title} /></div>
		<div class="meta">
			<div class="title ellipsis">{title}</div>
			<div class="sub ellipsis muted">{sub}</div>
		</div>
	</a>

	<div class="centre">
		<div class="transport">
			<button class="icon-btn" aria-label="Previous" onclick={() => app.send(cmd.skip(-1))} disabled={!app.now}>
				<Icon name="prev" />
			</button>
			<button
				class="icon-btn play"
				aria-label={app.playing ? 'Pause' : 'Play'}
				onclick={() => app.send(app.playing ? cmd.pause(true) : cmd.play())}
				disabled={!app.now}
			>
				<Icon name={app.playing ? 'pause' : 'play'} size={24} />
			</button>
			<button class="icon-btn" aria-label="Next" onclick={() => app.send(cmd.skip(1))} disabled={!app.now}>
				<Icon name="next" />
			</button>
		</div>
		<div class="progress">
			<span class="time">{duration(shownTime)}</span>
			<input
				type="range"
				min="0"
				max={Math.max(total, 1)}
				step="1"
				value={shownTime}
				disabled={!canSeek}
				aria-label="Position"
				oninput={(e) => (seeking = Number(e.currentTarget.value))}
				onchange={async (e) => {
					const t = Number(e.currentTarget.value);
					await app.seek(t);
					seeking = null;
				}}
			/>
			<span class="time">{duration(total)}</span>
		</div>
	</div>

	<div class="right">
		<label class="volume">
			<Icon name={vol > 0 ? 'volume' : 'mute'} size={20} />
			<span class="sr-only">Volume</span>
			<input type="range" min="0" max="100" value={vol} oninput={(e) => setVolume(Number(e.currentTarget.value))} disabled={!app.now} />
		</label>
		<div class="switch">
			<button class="btn player-btn" onclick={() => (switcher = !switcher)} aria-expanded={switcher}>
				<Icon name="speaker" size={18} />
				<span class="ellipsis">{textOf(app.current?.name) || 'Choose player'}</span>
			</button>
			{#if switcher}
				<div class="pop"><PlayerSwitcher onclose={() => (switcher = false)} /></div>
			{/if}
		</div>
	</div>
</footer>

<style>
	.bar {
		position: fixed;
		left: 0;
		right: 0;
		bottom: var(--tabs-h);
		height: var(--bar-h);
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 2fr) minmax(0, 1fr);
		align-items: center;
		gap: 1rem;
		padding: 0 1rem;
		background: color-mix(in srgb, var(--bg-raised) 92%, transparent);
		backdrop-filter: blur(14px);
		border-top: 1px solid var(--line);
		z-index: 20;
	}
	.now {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-width: 0;
	}
	.thumb {
		width: 52px;
		flex: none;
		box-shadow: var(--shadow);
		border-radius: var(--radius-sm);
	}
	.meta {
		min-width: 0;
	}
	.title {
		font-weight: 600;
	}
	.sub {
		font-size: 0.85rem;
	}
	.centre {
		display: grid;
		justify-items: center;
		gap: 0.1rem;
	}
	.transport {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	.play {
		background: var(--fg);
		color: var(--bg);
		width: 44px;
		height: 44px;
	}
	.play:hover {
		background: var(--fg);
		transform: scale(1.04);
	}
	.progress {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		width: min(100%, 560px);
	}
	.time {
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
		min-width: 3.2em;
		text-align: center;
	}
	.right {
		display: flex;
		align-items: center;
		justify-content: flex-end;
		gap: 0.75rem;
		min-width: 0;
	}
	.volume {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		width: 140px;
		color: var(--fg-muted);
	}
	.switch {
		position: relative;
		min-width: 0;
	}
	.player-btn {
		max-width: 200px;
	}
	.pop {
		position: absolute;
		right: 0;
		bottom: calc(100% + 10px);
	}

	/* Phones: a compact bar with title and play/pause; the rest is on the Now playing page. */
	@media (max-width: 760px) {
		.bar {
			grid-template-columns: minmax(0, 1fr) auto;
			gap: 0.5rem;
			padding: 0 0.5rem 0 0.75rem;
		}
		.centre {
			order: 2;
		}
		.progress,
		.volume,
		.transport .icon-btn:not(.play) {
			display: none;
		}
		.right {
			display: none;
		}
		.thumb {
			width: 46px;
		}
	}
</style>
