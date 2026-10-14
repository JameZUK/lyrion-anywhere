<script lang="ts">
	// Car mode (wave2-design.md §5.1): a driving screen with huge art, three giant buttons
	// (previous, play/pause, next; holding play/pause stops), Start radio from the current
	// track, a large elapsed time, and one level of Favourites and Stations. High contrast in
	// every theme, no visualiser, every target at least 76 px, and the screen kept on while
	// playing. Opened from Now playing; Exit (or Escape) goes back.
	import { goto } from '$app/navigation';
	import { api, app } from '$lib/state/app.svelte';
	import { me } from '$lib/state/me';
	import * as cmd from '$lib/lms/commands';
	import { controls } from '$lib/player/controls.svelte';
	import { duration, isRemote, textOf } from '$lib/lms/format';
	import type { FavouritesResult, MenuItem } from '$lib/lms/types';
	import type { Station } from '$lib/api/me';
	import { activeStation } from '$lib/stations/active.svelte';
	import { radioFromTrack } from '$lib/lms/discovery';
	import { ScreenWake } from '$lib/player/wake-lock';
	import { haptics } from '$lib/haptics/haptics.svelte';
	import { inDialog, typing } from '$lib/keys/shortcuts';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';

	/** Holding play/pause this long stops. */
	const LONG_PRESS_MS = 700;
	/** One level only: the first this many favourites and stations. */
	const LIST_MAX = 40;

	const track = $derived(app.now?.playlist_loop?.[0]);
	const title = $derived(textOf(track?.title ?? app.now?.current_title));
	const artist = $derived(textOf(track?.artist));
	const trackId = $derived(typeof track?.id === 'number' && track.id > 0 && !isRemote(track.remote) ? track.id : null);
	const total = $derived(Number(app.now?.duration ?? track?.duration ?? 0));

	let view = $state<'playing' | 'favourites' | 'stations'>('playing');
	let message = $state('');
	let busy = $state(false);

	// --- the screen stays on while playing -------------------------------------------------
	const wake = new ScreenWake();
	$effect(() => {
		wake.set(app.playing);
	});
	$effect(() => () => wake.destroy());

	// --- play/pause; holding it stops --------------------------------------------------------
	let pressTimer: ReturnType<typeof setTimeout> | undefined;
	let held = false;
	function pressStart(e: PointerEvent) {
		if (e.pointerType === 'mouse' && e.button !== 0) return;
		held = false;
		clearTimeout(pressTimer);
		pressTimer = setTimeout(() => {
			held = true;
			void stop();
		}, LONG_PRESS_MS);
	}
	function pressEnd() {
		clearTimeout(pressTimer);
	}
	function playPause(e: MouseEvent) {
		if (held) {
			held = false;
			e.preventDefault();
			return;
		}
		if (!app.now || !g.playPause) return;
		controls.toggle();
	}
	/** Which controls work now: out of reach, only what this device can do by itself. */
	const g = $derived(controls.gates);
	async function stop() {
		haptics.tick('press');
		await app.send(cmd.stop());
		message = 'Stopped';
	}
	function contextStop(e: MouseEvent) {
		// The context-menu key, Shift+F10, or a touch long-press the browser reports.
		e.preventDefault();
		if (held) return;
		held = true;
		clearTimeout(pressTimer);
		void stop();
	}
	$effect(() => () => clearTimeout(pressTimer));

	function skip(by: 1 | -1) {
		if (!(by > 0 ? g.next : g.previous)) return;
		haptics.tick('skip');
		if (by > 0) controls.next();
		else controls.previous();
	}

	// --- Start radio ---------------------------------------------------------------------------
	async function radio() {
		if (trackId === null || !app.playerId || busy) return;
		busy = true;
		message = 'Starting radio…';
		try {
			message = (await radioFromTrack((p, c) => api.rpc(p, c), app.playerId, trackId)) || 'Radio started';
			app.queueVersion++;
			app.soon();
		} finally {
			busy = false;
		}
	}

	// --- one level of Favourites and Stations ---------------------------------------------------
	let favourites = $state<MenuItem[] | null>(null);
	let stations = $state<Station[] | null>(null);
	let listError = $state('');
	async function open(v: 'favourites' | 'stations') {
		view = v;
		listError = '';
		try {
			if (v === 'favourites' && favourites === null) {
				const r = await api.rpc<FavouritesResult>('', cmd.favourites(0, LIST_MAX));
				// Only what plays at once: folders would need deeper lists.
				favourites = (r.loop_loop ?? []).filter((f) => typeof f.id === 'string' && Number(f.isaudio) === 1).slice(0, LIST_MAX);
			}
			if (v === 'stations' && stations === null) stations = (await me.stations()).filter((s) => !s.broken).slice(0, LIST_MAX);
		} catch (e) {
			listError = (e as Error).message;
		}
	}
	async function playFavourite(f: MenuItem) {
		view = 'playing';
		await app.send(cmd.playFavourite(f.id!, 'play'));
		message = `Playing ${textOf(f.name)}`;
	}
	async function playStation(s: Station) {
		view = 'playing';
		const ok = await activeStation.start(s);
		message = ok ? `Playing ${s.name}` : activeStation.error;
	}

	function exit() {
		void goto('#/');
	}
	function onkeydown(e: KeyboardEvent) {
		if (e.key !== 'Escape' || e.defaultPrevented || typing(e.target) || inDialog(e.target)) return;
		e.preventDefault();
		if (view !== 'playing') view = 'playing';
		else exit();
	}
</script>

<svelte:window {onkeydown} />

<section class="car" aria-labelledby="car-title" data-car>
	<header class="top">
		<h1 id="car-title" class="sr-only">Car mode</h1>
		<span class="who">{app.current ? app.nameOf(app.playerId) : 'No player'}</span>
		<button class="exit" onclick={exit} data-car-exit><Icon name="close" size={30} /><span>Exit</span></button>
	</header>

	<p class="sr-only" role="status" aria-live="polite" data-car-status>{message}</p>

	{#if view === 'playing'}
		<div class="stage">
			<div class="art">
				<Art path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 800)} alt={title ? `Cover of ${textOf(track?.album)}` : ''} label={title} rounded="12px" eager />
			</div>
			<div class="about">
				<p class="title">{title || (app.current ? 'Nothing playing' : 'Choose a player on Now playing')}</p>
				{#if artist}<p class="artist">{artist}</p>{/if}
				<p class="time" aria-label={`Elapsed ${duration(app.elapsed)}${total > 0 ? ` of ${duration(total)}` : ''}`} data-car-time>
					<span class="elapsed">{duration(app.elapsed)}</span>{#if total > 0}<span class="of"> / {duration(total)}</span>{/if}
				</p>
				{#if message}<p class="note" aria-hidden="true">{message}</p>{/if}
			</div>
		</div>
	{:else}
		<div class="list" role="region" aria-label={view === 'favourites' ? 'Favourites' : 'Stations'} data-car-list={view}>
			<h2>{view === 'favourites' ? 'Favourites' : 'Stations'}</h2>
			{#if listError}
				<p class="note">{listError}</p>
			{:else if view === 'favourites'}
				{#if favourites === null}<p class="note">Loading…</p>
				{:else if !favourites.length}<p class="note">No favourites that play at once.</p>
				{:else}
					<ul>
						{#each favourites as f (f.id)}
							<li><button class="item" onclick={() => playFavourite(f)} data-car-favourite={f.id}>{textOf(f.name)}</button></li>
						{/each}
					</ul>
				{/if}
			{:else if stations === null}<p class="note">Loading…</p>
			{:else if !stations.length}<p class="note">No stations yet. Make one on the Stations page.</p>
			{:else}
				<ul>
					{#each stations as s (s.id)}
						<li><button class="item" onclick={() => playStation(s)} data-car-station={s.id}>{s.name}</button></li>
					{/each}
				</ul>
			{/if}
		</div>
	{/if}

	<div class="transport" role="group" aria-label="Playback">
		<button class="big" aria-label="Previous" onclick={() => skip(-1)} disabled={!g.previous} data-car-prev><Icon name="prev" size={56} /></button>
		<button
			class="big play"
			aria-label={app.playing ? 'Pause' : 'Play'}
			aria-describedby="car-hold"
			onpointerdown={pressStart}
			onpointerup={pressEnd}
			onpointerleave={pressEnd}
			onpointercancel={pressEnd}
			onclick={playPause}
			oncontextmenu={contextStop}
			disabled={!g.playPause}
			data-car-play
		><Icon name={app.playing ? 'pause' : 'play'} size={64} /></button>
		<button class="big" aria-label="Next" onclick={() => skip(1)} disabled={!g.next} data-car-next><Icon name="next" size={56} /></button>
	</div>
	<p class="hold" id="car-hold">Hold play to stop</p>

	<nav class="more" aria-label="Car mode">
		<button class="wide" onclick={radio} disabled={trackId === null || busy || !g.other} data-car-radio><Icon name="radio" size={30} /><span>Start radio</span></button>
		<button class="wide" aria-pressed={view === 'favourites'} onclick={() => (view === 'favourites' ? (view = 'playing') : open('favourites'))} data-car-favourites>
			<Icon name="heart" size={30} /><span>Favourites</span>
		</button>
		<button class="wide" aria-pressed={view === 'stations'} onclick={() => (view === 'stations' ? (view = 'playing') : open('stations'))} data-car-stations>
			<Icon name="shuffle" size={30} /><span>Stations</span>
		</button>
	</nav>
</section>

<style>
	/* High contrast whatever the theme: white on black, bold type, thick outlines. */
	.car {
		--car-bg: #000;
		--car-fg: #fff;
		--car-muted: #d6d6d6;
		--car-raised: #1c1c1c;
		--car-line: #fff;
		position: fixed;
		inset: 0;
		z-index: 40;
		display: grid;
		grid-template-rows: auto minmax(0, 1fr) auto auto auto;
		gap: 0.75rem;
		padding: max(0.75rem, env(safe-area-inset-top)) max(1rem, env(safe-area-inset-right)) max(0.75rem, env(safe-area-inset-bottom)) max(1rem, env(safe-area-inset-left));
		background: var(--car-bg);
		color: var(--car-fg);
		overflow: hidden;
	}
	.top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
	}
	.who {
		font-size: 1.25rem;
		font-weight: 700;
		color: var(--car-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	button {
		font: inherit;
		color: var(--car-fg);
		background: var(--car-raised);
		border: 3px solid var(--car-line);
		border-radius: 18px;
		cursor: pointer;
	}
	button:disabled {
		opacity: 0.45;
		cursor: default;
	}
	button:focus-visible {
		outline: 4px solid #ffd400;
		outline-offset: 3px;
	}
	.exit {
		display: inline-flex;
		align-items: center;
		gap: 0.5rem;
		min-width: 76px;
		min-height: 76px;
		padding: 0 1.25rem;
		font-size: 1.4rem;
		font-weight: 800;
	}
	.stage {
		min-height: 0;
		display: grid;
		grid-template-columns: minmax(0, auto) minmax(0, 1fr);
		grid-template-rows: minmax(0, 1fr);
		align-items: center;
		gap: 1.5rem;
		overflow: hidden;
	}
	.art {
		height: 100%;
		max-width: 55vw;
		aspect-ratio: 1;
		min-height: 0;
		justify-self: start;
	}
	.about {
		min-width: 0;
	}
	.title {
		margin: 0;
		font-size: clamp(1.4rem, min(4.5vw, 7vh), 3rem);
		font-weight: 800;
		line-height: 1.1;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}
	.artist {
		margin: 0.4rem 0 0;
		font-size: clamp(1.1rem, min(3vw, 5vh), 2rem);
		font-weight: 650;
		color: var(--car-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.time {
		margin: 0.75rem 0 0;
		font-variant-numeric: tabular-nums;
		font-weight: 800;
	}
	.elapsed {
		font-size: clamp(2.25rem, min(8vw, 12vh), 5rem);
	}
	.of {
		font-size: clamp(1.25rem, 3vw, 2rem);
		color: var(--car-muted);
	}
	.note {
		margin: 0.5rem 0 0;
		font-size: 1.25rem;
		color: var(--car-muted);
	}
	.list {
		min-height: 0;
		overflow-y: auto;
		overscroll-behavior: contain;
	}
	.list h2 {
		margin: 0 0 0.5rem;
		font-size: 1.6rem;
	}
	.list ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 18rem), 1fr));
		gap: 0.6rem;
	}
	.item {
		width: 100%;
		min-height: 76px;
		padding: 0.5rem 1rem;
		font-size: 1.4rem;
		font-weight: 700;
		text-align: left;
	}
	.transport {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1.3fr) minmax(0, 1fr);
		gap: 0.75rem;
	}
	.big {
		display: grid;
		place-items: center;
		min-width: 76px;
		min-height: 96px;
	}
	.play {
		background: var(--car-fg);
		color: var(--car-bg);
	}
	.hold {
		margin: -0.25rem 0 0;
		text-align: center;
		font-size: 1rem;
		color: var(--car-muted);
	}
	.more {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 0.75rem;
	}
	.wide {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 0.6rem;
		min-height: 76px;
		padding: 0 0.5rem;
		font-size: 1.3rem;
		font-weight: 800;
	}
	.wide[aria-pressed='true'] {
		background: var(--car-fg);
		color: var(--car-bg);
	}
	/* Portrait phones: the art above the words. */
	@media (max-aspect-ratio: 1/1) {
		.stage {
			grid-template-columns: minmax(0, 1fr);
			grid-template-rows: minmax(0, 1fr) auto;
			justify-items: center;
			text-align: center;
			gap: 0.75rem;
		}
		.art {
			justify-self: center;
			max-width: 100%;
		}
		.wide {
			flex-direction: column;
			gap: 0.2rem;
			padding: 0.4rem 0.25rem;
			font-size: 1.05rem;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.car * {
			transition: none !important;
			animation: none !important;
		}
	}
</style>
