<script lang="ts">
	import { go } from '$lib/nav';
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { coverPath } from '$lib/lms/commands';
	import { duration, textOf } from '$lib/lms/format';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import OutputButton from './OutputButton.svelte';
	import PlayingElsewhere from './PlayingElsewhere.svelte';
	import { playerLabel } from '$lib/lms/sync';
	import { device } from '$lib/player/device.svelte';
	import { DEVICE_BUTTONS_NOTE } from '$lib/player/volume';
	import { LONG_PRESS_MS, MiniGesture, miniKey, type MiniAction } from '$lib/player/gesture';
	import { formatLeft, sleepLeft } from '$lib/lms/sleep';
	import { haptics } from '$lib/haptics/haptics.svelte';
	import { controls } from '$lib/player/controls.svelte';

	let seeking = $state<number | null>(null);
	let volume = $state<number | null>(null);
	let volTimer: ReturnType<typeof setTimeout> | undefined;

	const track = $derived(app.now?.playlist_loop?.[0]);
	const total = $derived(app.now?.duration ?? track?.duration ?? 0);
	const shownTime = $derived(seeking ?? app.elapsed);
	const vol = $derived(volume ?? app.now?.['mixer volume'] ?? 0);
	const title = $derived(textOf(track?.title ?? app.now?.current_title) || (app.current ? 'Nothing playing' : 'No player chosen'));
	const sub = $derived(app.current ? [textOf(track?.artist), textOf(track?.album)].filter(Boolean).join(' · ') : 'Choose a player');
	/** Which player is in use, when it isn't this device (Wave 3.5: no separate strip for it). */
	const on = $derived(app.remote ? playerLabel(app.playerId, app.now, app.players) || app.nameOf(app.playerId) || 'another player' : '');
	/** Which controls work now (all of them while the server is in reach). */
	const g = $derived(controls.gates);
	const canSeek = $derived(total > 0 && app.now?.can_seek !== 0 && g.seek);
	const sleepIn = $derived(sleepLeft(app.now));
	const thumb = $derived(coverPath(track?.coverid ?? track?.artwork_track_id, 120));

	// --- gestures on the mini player (gesture.ts): swipe for next/previous, up to open, a
	// long-press for Play on…; each also has a button and a keyboard shortcut ---------------
	let playOn = $state(false);
	let nowEl = $state<HTMLAnchorElement>();
	const gesture = new MiniGesture();
	let pressTimer: ReturnType<typeof setTimeout> | undefined;
	let follow = $state<{ dx: number; dy: number } | null>(null);
	/** Set when a gesture ended the press, so the click that follows doesn't also open. */
	let swallow = false;

	function act(a: MiniAction) {
		if (a === 'none') return;
		if (a === 'next' || a === 'previous') {
			if (!(a === 'next' ? g.next : g.previous)) return;
			haptics.tick('skip');
			if (a === 'next') controls.next();
			else controls.previous();
		} else if (a === 'long-press') {
			haptics.tick('press');
			playOn = true;
		} else {
			void go('/');
		}
	}

	function pdown(e: PointerEvent) {
		if (e.pointerType === 'mouse' && e.button !== 0) return;
		swallow = false;
		gesture.setWidth(nowEl?.getBoundingClientRect().width ?? 0);
		gesture.down(e.clientX, e.clientY, e.timeStamp);
		clearTimeout(pressTimer);
		pressTimer = setTimeout(() => {
			if (gesture.tick(e.timeStamp + LONG_PRESS_MS) === 'long-press') {
				swallow = true;
				act('long-press');
			}
		}, LONG_PRESS_MS);
	}
	function pmove(e: PointerEvent) {
		if (gesture.move(e.clientX, e.clientY)) {
			clearTimeout(pressTimer);
			(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
			const d = gesture.drag;
			follow = d ? { dx: d.axis === 'x' ? d.dx : 0, dy: d.axis === 'y' ? Math.min(0, d.dy) : 0 } : null;
		} else if (gesture.state !== 'pressed') {
			clearTimeout(pressTimer);
		}
	}
	function pup(e: PointerEvent) {
		clearTimeout(pressTimer);
		follow = null;
		const a = gesture.up(e.clientX, e.clientY, e.timeStamp);
		if (a === 'tap' || a === 'none') return; // a tap is the link's own click
		swallow = true;
		act(a);
	}
	function pcancel() {
		clearTimeout(pressTimer);
		follow = null;
		gesture.cancel();
	}
	function onclick(e: MouseEvent) {
		if (swallow) {
			swallow = false;
			e.preventDefault();
		}
	}
	function onkeydown(e: KeyboardEvent) {
		const a = miniKey(e);
		if (a === 'none' || (a === 'open' && e.key === 'Enter')) return; // Enter: the link itself
		e.preventDefault();
		act(a);
	}
	function oncontextmenu(e: MouseEvent) {
		// A right-click (or a touch long-press the browser reports): Play on…
		e.preventDefault();
		if (!swallow) act('long-press');
		swallow = true;
	}

	function setVolume(v: number) {
		haptics.volume(vol, v);
		volume = v;
		clearTimeout(volTimer);
		volTimer = setTimeout(async () => {
			await app.send(cmd.volume(v));
			volume = null;
		}, 150);
	}
</script>

<footer class="bar" aria-label="Player">
	{#if thumb}<div class="tint" aria-hidden="true"><Art path={thumb} alt="" rounded="0" /></div>{/if}
	<div class="strip"><PlayingElsewhere compact /></div>
	<a
		class="now"
		href="#/"
		aria-label={`Now playing: ${title}${on ? `, on ${on}` : ''}${sleepIn !== null ? `. Sleep timer: ${formatLeft(sleepIn)}` : ''}`}
		aria-describedby="mini-hint"
		bind:this={nowEl}
		style:transform={follow ? `translate(${follow.dx * 0.5}px, ${follow.dy * 0.4}px)` : undefined}
		onpointerdown={pdown}
		onpointermove={pmove}
		onpointerup={pup}
		onpointercancel={pcancel}
		{onclick}
		{onkeydown}
		{oncontextmenu}
		data-mini-player
	>
		<div class="thumb"><Art path={thumb} alt="" label={title} /></div>
		<div class="meta">
			<div class="title ellipsis">{title}</div>
			<div class="sub ellipsis muted">
				{#if sleepIn !== null}<span class="moon" title={`Sleep timer: ${formatLeft(sleepIn)}`}><Icon name="moon" size={13} /></span>{/if}{#if on}<span class="on" data-mini-on>on {on}</span>{#if sub}{' · '}{/if}{/if}<span class:pick={!app.current}>{sub}</span>
			</div>
		</div>
	</a>
	<span id="mini-hint" class="sr-only">Swipe left or right, or press Alt and an arrow key, for the next or previous track. Long-press, or press Shift and F10, for Play on.</span>

	<div class="centre">
		<div class="transport">
			<button class="icon-btn" aria-label="Previous" onclick={() => controls.previous()} disabled={!g.previous} {...controls.why(!g.previous)}>
				<Icon name="prev" />
			</button>
			<button class="icon-btn play" aria-label={app.playing ? 'Pause' : 'Play'} onclick={() => controls.toggle()} disabled={!g.playPause} {...controls.why(!g.playPause)}>
				<Icon name={app.playing ? 'pause' : 'play'} size={24} />
			</button>
			<button class="icon-btn" aria-label="Next" onclick={() => controls.next()} disabled={!g.next} {...controls.why(!g.next)}>
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
					await controls.seek(t);
					seeking = null;
				}}
			/>
			<span class="time">{duration(total)}</span>
		</div>
	</div>

	<div class="right">
		{#if device.buttonsOnly(app.playerId)}
			<p class="volume note muted" data-volume-note>{DEVICE_BUTTONS_NOTE}</p>
		{:else}
			<label class="volume">
				<Icon name={vol > 0 ? 'volume' : 'mute'} size={20} />
				<span class="sr-only">Volume</span>
				<input type="range" min="0" max="100" value={vol} oninput={(e) => setVolume(Number(e.currentTarget.value))} disabled={!g.volume} />
			</label>
		{/if}
		<OutputButton bind:open={playOn} />
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
		touch-action: none;
		user-select: none;
		-webkit-touch-callout: none;
	}
	/* A faint wash of the cover behind the bar (its surface stays at least 94% opaque, so
	   text keeps its contrast: tests/ambient.test.ts). */
	.tint {
		position: absolute;
		inset: 0;
		overflow: hidden;
		opacity: 0.06;
		pointer-events: none;
		z-index: -1;
	}
	.tint :global(.art) {
		position: absolute;
		inset: -40%;
		width: auto;
		aspect-ratio: auto;
		filter: blur(30px) saturate(1.4);
	}
	.moon {
		display: inline-flex;
		vertical-align: -2px;
		margin-right: 0.3rem;
		color: var(--accent);
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
	/* The player in use, first so it is never cut off: "on Lounge · Artist · Album". */
	.on,
	.pick {
		color: var(--accent);
		font-weight: 600;
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
	.volume.note {
		margin: 0;
		font-size: 0.75rem;
		line-height: 1.2;
	}
	/* "Playing on …": a strip along the top edge of the bar. */
	.strip {
		position: absolute;
		left: 0;
		right: 0;
		bottom: 100%;
	}

	/* Phones: a compact bar with title and play/pause; the rest is on the Now playing page. */
	@media (max-width: 760px) {
		.bar {
			grid-template-columns: minmax(0, 1fr) auto auto;
			gap: 0.25rem;
			padding: 0 0.5rem 0 0.75rem;
		}
		.centre {
			order: 2;
		}
		.right {
			order: 3;
		}
		.progress,
		.volume {
			display: none;
		}
		/* Previous and next are swipes here; the buttons stay for screen readers and keys. */
		.transport .icon-btn:not(.play) {
			position: absolute;
			width: 1px;
			height: 1px;
			padding: 0;
			overflow: hidden;
			clip: rect(0 0 0 0);
		}
		.thumb {
			width: 42px;
		}
		/* The bar's backdrop filter makes it the frame for the Play on… sheet. */
		.bar {
			--pop-bottom: calc(100% + 8px);
		}
	}
</style>
