<script lang="ts">
	// Now playing on phones and narrow windows (up to 840 px wide). One column the height of
	// the screen above the tab bar, which never scrolls: a slim top row (close, "Playing on …",
	// ⋮), the hero (a pager of Cover, Lyrics and Track info, which takes whatever height is
	// left, the cover shrinking to fit), the title with its rating, artist · album and the
	// quality badge, the seek bar, the transport, Up next, and a row of four actions (Sleep,
	// Queue, Visualiser, Volume). Lyrics and track info scroll inside their own page.
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, isRemote, textOf } from '$lib/lms/format';
	import { badgeText, quality } from '$lib/lms/quality';
	import { formatLeft, sleepLeft } from '$lib/lms/sleep';
	import { device } from '$lib/player/device.svelte';
	import { controls } from '$lib/player/controls.svelte';
	import { isThisDevice } from '$lib/player/outputs';
	import { nowPlaying } from '$lib/nowplaying/store.svelte';
	import { STYLES, STYLE_LABELS, type VisualStyle } from '$lib/visual/draw';
	import { GLOW_REST, type GlowStyle } from '$lib/visual/glow';
	import { lyrics } from '$lib/state/lyrics';
	import { reducedMotion } from '$lib/theme/motion';
	import { back } from '$lib/nav';
	import { fade } from 'svelte/transition';
	import Ambient from './Ambient.svelte';
	import Art from './Art.svelte';
	import ContextMenu from './ContextMenu.svelte';
	import DeviceWait from './DeviceWait.svelte';
	import GlowHalo from './GlowHalo.svelte';
	import Icon from './Icon.svelte';
	import LyricsLines from './LyricsLines.svelte';
	import NowPlayingInfo from './NowPlayingInfo.svelte';
	import NowPlayingPager from './NowPlayingPager.svelte';
	import PlayerSwitcher from './PlayerSwitcher.svelte';
	import QueuePeek from './QueuePeek.svelte';
	import RatingControl from './RatingControl.svelte';
	import SeekBar from './SeekBar.svelte';
	import Sheet from './Sheet.svelte';
	import ShuffleButton from './ShuffleButton.svelte';
	import SignalPath from './SignalPath.svelte';
	import SleepSheet from './SleepSheet.svelte';
	import UpNext from './UpNext.svelte';
	import Visualiser from './Visualiser.svelte';
	import VisualiserButton from './VisualiserButton.svelte';
	import VolumePopover from './VolumePopover.svelte';

	let { reduce = false }: { reduce?: boolean } = $props();

	// --- what is playing ------------------------------------------------------------------------
	const track = $derived(app.now?.playlist_loop?.[0]);
	const total = $derived(app.now?.duration ?? track?.duration ?? 0);
	let seeking = $state<number | null>(null);
	const shown = $derived(seeking ?? app.elapsed);
	const title = $derived(textOf(track?.title ?? app.now?.current_title));
	const artist = $derived(textOf(track?.artist));
	const album = $derived(textOf(track?.album));
	const artistHref = $derived(cmd.link('artist', track?.artist_id));
	const albumHref = $derived(cmd.link('album', track?.album_id));
	const repeat = $derived(Number(app.now?.['playlist repeat'] ?? 0));
	/** Which controls work now (all of them while the server is in reach). */
	const g = $derived(controls.gates);
	const badge = $derived(quality(track));
	const sleepIn = $derived(sleepLeft(app.now));
	const coverOf = (size: number) => cmd.coverPath(track?.coverid ?? track?.artwork_track_id, size);
	/** Art and title cross-fade between tracks (not under "reduce motion"). */
	const trackKey = $derived(`${track?.id ?? ''}|${title}`);
	const swap = { duration: reducedMotion() ? 0 : 350 };
	/** A library track (not a stream): it has lyrics, a rating and a track menu. */
	const libraryId = $derived(typeof track?.id === 'number' && track.id > 0 && !isRemote(track.remote) ? track.id : null);
	$effect(() => {
		void lyrics.load(libraryId);
	});

	// --- the player -------------------------------------------------------------------------------
	/** This browser is the player (the visualiser needs its audio). */
	const local = $derived(!!app.current && isThisDevice(app.playerId) && (device.state === 'connected' || device.state === 'reconnecting'));
	const chip = $derived.by(() => {
		if (app.moving) return `Moving to ${app.moving}…`;
		if (!app.current) return 'Choose a player';
		return `Playing on ${isThisDevice(app.playerId) ? 'this device' : app.nameOf(app.playerId) || 'another player'}`;
	});
	let playOn = $state(false);

	// --- the pager ----------------------------------------------------------------------------------
	const page = $derived(nowPlaying.layout);

	// --- the visualiser ---------------------------------------------------------------------------
	const visualising = $derived(local && nowPlaying.visualiser && !reduce && page === 'art');
	/** Glow: a halo in the cover's colours behind it (still, at rest, under reduced motion). */
	const glowing = $derived(local && nowPlaying.visualiser && nowPlaying.style === 'glow' && page === 'art');
	let section = $state<HTMLElement>();
	function onglow(g: GlowStyle) {
		const s = section?.style;
		if (!s) return;
		s.setProperty('--glow-radius', g.radius.toFixed(3));
		s.setProperty('--glow-opacity', g.opacity.toFixed(3));
		s.setProperty('--glow-cover', g.cover.toFixed(4));
	}
	$effect(() => {
		if (!visualising || nowPlaying.style !== 'glow') onglow(GLOW_REST);
	});

	/** A word that shows for a moment (the style's name), and is read out. */
	let toast = $state('');
	let toastTimer: ReturnType<typeof setTimeout> | undefined;
	function say(text: string) {
		toast = text;
		clearTimeout(toastTimer);
		toastTimer = setTimeout(() => (toast = ''), 1400);
	}
	$effect(() => () => clearTimeout(toastTimer));

	function toggleVisualiser() {
		const on = !nowPlaying.visualiser;
		nowPlaying.setVisualiser(on);
		if (on && page !== 'art') nowPlaying.setLayout('art');
		say(on ? STYLE_LABELS[nowPlaying.style] : 'Visualiser off');
	}
	function chooseStyle(s: VisualStyle) {
		closeSheet();
		nowPlaying.setStyle(s);
		nowPlaying.setVisualiser(true);
		if (page !== 'art') nowPlaying.setLayout('art');
		say(STYLE_LABELS[s]);
	}
	/** A swipe across the visualiser. */
	function swipeStyle(s: VisualStyle) {
		nowPlaying.setStyle(s);
		say(STYLE_LABELS[s]);
	}
	function switchOff() {
		closeSheet();
		nowPlaying.setVisualiser(false);
		say('Visualiser off');
	}

	// --- sheets -------------------------------------------------------------------------------------
	type SheetKind = 'more' | 'sleep' | 'path' | 'queue' | 'styles' | 'track';
	let sheet = $state<SheetKind | null>(null);
	/** Where focus goes back to when the sheet closes. */
	let returnTo: HTMLElement | null = null;
	function openSheet(kind: SheetKind, from?: Element | null) {
		const el = from ?? document.activeElement;
		// From inside another sheet (⋮ › Signal path): back to what opened that one.
		if (!sheet || !returnTo) returnTo = el instanceof HTMLElement && el !== document.body ? el : null;
		sheet = kind;
	}
	function closeSheet() {
		sheet = null;
		const el = returnTo;
		returnTo = null;
		queueMicrotask(() => {
			const a = document.activeElement;
			if ((!a || a === document.body || !a.isConnected) && el?.isConnected) el.focus();
		});
	}
	const visButton = () => section?.querySelector('[data-visualiser-button]') ?? null;
	const moreButton = () => section?.querySelector('[data-np-more]') ?? null;
</script>

<section class="np np-phone" bind:this={section} data-np-phone data-layout={page}>
	<Ambient path={coverOf(120)} />

	<header class="top">
		<button class="icon-btn" aria-label="Close Now playing" title="Close Now playing" onclick={() => void back('/home')} data-np-close><Icon name="chevron" size={24} /></button>
		<div class="playon">
			<button class="chip" class:remote={app.remote} aria-haspopup="dialog" aria-expanded={playOn} onclick={() => (playOn = !playOn)} data-play-on>
				<span class="dot" aria-hidden="true"></span><span class="chip-text">{chip}</span><Icon name="chevron" size={16} />
			</button>
			{#if playOn}<div class="pop"><PlayerSwitcher onclose={() => (playOn = false)} /></div>{/if}
		</div>
		<button class="icon-btn" aria-label="More options" aria-haspopup="dialog" onclick={() => openSheet('more')} data-np-more><Icon name="more" size={24} /></button>
	</header>

	<div class="hero">
		<NowPlayingPager page={page} onpage={(p) => nowPlaying.setLayout(p)} onpeek={() => app.current && openSheet('queue')} still={reduce}>
			{#snippet cover()}
				<div class="stage">
					<div class="art-box" class:glowing>
						{#if glowing}<GlowHalo path={coverOf(120)} playing={app.playing} still={reduce} />{/if}
						{#key trackKey}
							<div class="cover" in:fade={swap}>
								<Art path={coverOf(800)} alt={title ? `Cover of ${album}` : ''} label={title} rounded="var(--radius)" eager />
							</div>
						{/key}
						{#if visualising}
							{#if nowPlaying.style === 'glow'}
								<Visualiser style="glow" {onglow} />
							{:else}
								<div class="vis-over"><Visualiser style={nowPlaying.style} onstyle={swipeStyle} /></div>
							{/if}
						{/if}
					</div>
				</div>
			{/snippet}
			{#snippet lyrics()}
				<div class="scroll" data-lyrics-pane><LyricsLines compact /></div>
			{/snippet}
			{#snippet info()}
				<div class="scroll"><NowPlayingInfo compact /></div>
			{/snippet}
		</NowPlayingPager>
		<div class="toast" class:shown={!!toast} role="status" aria-live="polite" data-np-toast>{toast}</div>
	</div>

	<div class="meta">
		{#if !app.current && app.playerId && app.playerId === app.deviceMac}
			<DeviceWait />
		{:else if !app.current}
			<h1 class="title">Nothing to control yet</h1>
			<p class="sub muted">Choose a player with “Choose a player” at the top.</p>
		{:else if !track && !app.now?.current_title}
			<h1 class="title">Nothing playing</h1>
			<p class="sub muted">Pick an album to start. <a class="link" href="#/albums">Browse albums</a></p>
		{:else}
			<div class="title-row">
				{#key trackKey}<h1 class="title" in:fade={swap} title={title}>{title}</h1>{/key}
				{#if libraryId !== null}<div class="rating"><RatingControl track={libraryId} size={20} compact /></div>{/if}
			</div>
			<div class="sub">
				<span class="who muted">
					{#if artistHref}<a href={artistHref}>{artist}</a>{:else}{artist}{/if}{#if artist && album}{' · '}{/if}{#if albumHref}<a href={albumHref}>{album}</a>{:else}{album}{/if}
				</span>
				{#if badge}
					<button class="badge" class:hires={badge.hiRes} onclick={() => openSheet('path')} aria-haspopup="dialog" aria-label={`${badgeText(badge)}. Show the signal path`} data-quality-badge>{badgeText(badge)}</button>
				{/if}
			</div>
		{/if}
	</div>

	<div class="seek">
		<SeekBar
			trackId={libraryId}
			value={shown}
			{total}
			disabled={!total || app.now?.can_seek === 0 || !g.seek}
			oninput={(v) => (seeking = v)}
			onchange={async (v) => {
				await controls.seek(v);
				seeking = null;
			}}
		/>
		<div class="times muted"><span>{duration(shown)}</span><span>-{duration(Math.max(0, total - shown))}</span></div>
	</div>

	<div class="transport">
		<ShuffleButton value={app.now?.['playlist shuffle']} onchange={(m) => app.send(cmd.shuffle(m))} disabled={!g.other} />
		<button class="icon-btn big" aria-label="Previous" onclick={() => controls.previous()} disabled={!g.previous} {...controls.why(!g.previous)}><Icon name="prev" size={30} /></button>
		<button class="icon-btn play" aria-label={app.playing ? 'Pause' : 'Play'} onclick={() => controls.toggle()} disabled={!g.playPause} {...controls.why(!g.playPause)}>
			<Icon name={app.playing ? 'pause' : 'play'} size={34} />
		</button>
		<button class="icon-btn big" aria-label="Next" onclick={() => controls.next()} disabled={!g.next} {...controls.why(!g.next)}><Icon name="next" size={30} /></button>
		<button
			class="icon-btn"
			class:on={repeat > 0}
			aria-pressed={repeat > 0}
			aria-label={repeat === 1 ? 'Repeat: this track' : repeat === 2 ? 'Repeat: all' : 'Repeat: off'}
			onclick={() => app.send(cmd.repeat(((repeat + 1) % 3) as 0 | 1 | 2))}
			disabled={!g.other}
		>
			<Icon name="repeat" />{#if repeat === 1}<span class="one">1</span>{/if}
		</button>
	</div>

	<div class="next"><UpNext /></div>

	<nav class="actions" aria-label="Now playing actions">
		<button
			class="np-act"
			class:set={sleepIn !== null}
			onclick={() => openSheet('sleep')}
			aria-haspopup="dialog"
			aria-label={sleepIn !== null ? `Sleep timer: stops in ${formatLeft(sleepIn)}` : 'Sleep timer'}
			disabled={!app.current || !g.other}
			data-sleep-button
		><Icon name="moon" size={24} /><span class="np-act-label">{sleepIn !== null ? formatLeft(sleepIn) : 'Sleep'}</span></button>
		<button class="np-act" onclick={() => openSheet('queue')} aria-haspopup="dialog" aria-label="Queue" disabled={!app.current} data-queue-peek><Icon name="queue" size={24} /><span class="np-act-label">Queue</span></button>
		{#if local}
			<VisualiserButton on={nowPlaying.visualiser} style={STYLE_LABELS[nowPlaying.style]} {reduce} ontoggle={toggleVisualiser} onchoose={() => openSheet('styles', visButton())} />
		{/if}
		<VolumePopover disabled={!g.volume} />
	</nav>

	{#if sheet === 'more'}
		<Sheet title="More" onclose={closeSheet}>
			<div class="rows" data-more-sheet>
				{#if libraryId !== null}
					<!-- LMS's own menu for the track: it needs the server (the rest here works without). -->
					<button class="row" onclick={() => openSheet('track', moreButton())} {...controls.needsServer()} data-track-menu><Icon name="note" /><span>This track<small>Add to a playlist, go to the album and more</small></span></button>
				{/if}
				{#if app.current}
					<a class="row" href="#/car" data-car-link><Icon name="car" /><span>Car mode<small>Big buttons for driving</small></span></a>
					<a class="row" href="#/alarms" data-alarms-link><Icon name="alarm" /><span>Alarms<small>Wake to music on this player</small></span></a>
				{/if}
				<a class="row" href="#/rooms" data-rooms-link><Icon name="rooms" /><span>All rooms<small>Every player at a glance</small></span></a>
				<button class="row" onclick={() => openSheet('path', moreButton())} disabled={!track} data-signal-path-row><Icon name="signal" /><span>Signal path<small>From the file to your ears</small></span></button>
				{#if local}
					<button class="row" onclick={() => openSheet('styles', moreButton())} disabled={reduce} data-visualiser-style-row>
						<Icon name="visualiser" /><span>Visualiser style<small>{reduce ? 'Off while your device asks for reduced motion' : nowPlaying.visualiser ? STYLE_LABELS[nowPlaying.style] : `Off (${STYLE_LABELS[nowPlaying.style]} when on)`}</small></span>
					</button>
				{/if}
			</div>
		</Sheet>
	{:else if sheet === 'styles'}
		<Sheet title="Visualiser style" onclose={closeSheet}>
			<div class="rows" role="group" aria-label="Visualiser style" data-style-sheet>
				{#each STYLES as s (s)}
					<button class="row" aria-pressed={nowPlaying.visualiser && nowPlaying.style === s} onclick={() => chooseStyle(s)} data-style-choice={s}>
						<span class="tick" aria-hidden="true">{nowPlaying.visualiser && nowPlaying.style === s ? '✓' : ''}</span><span>{STYLE_LABELS[s]}</span>
					</button>
				{/each}
				{#if nowPlaying.visualiser}
					<button class="row" onclick={switchOff} data-style-off><span class="tick" aria-hidden="true"></span><span>Switch off</span></button>
				{/if}
			</div>
		</Sheet>
	{:else if sheet === 'sleep'}
		<SleepSheet onclose={closeSheet} />
	{:else if sheet === 'path'}
		<SignalPath onclose={closeSheet} />
	{:else if sheet === 'queue'}
		<QueuePeek onclose={closeSheet} />
	{:else if sheet === 'track' && libraryId !== null}
		<ContextMenu kind="track" id={libraryId} {title} onclose={closeSheet} />
	{/if}
</section>

<style>
	/* The column fills the content area above the tab bar (the layout makes that area exactly
	   the visible height, 100dvh) and never scrolls: only the hero can shrink. */
	.np-phone {
		flex: 1 1 0;
		min-height: 0;
		width: 100%;
		max-width: 560px;
		margin-inline: auto;
		display: flex;
		flex-direction: column;
		gap: 4px;
		overflow: hidden;
		padding: max(4px, env(safe-area-inset-top)) 16px 6px;
		box-sizing: border-box;
	}
	.top {
		flex: none;
		height: 48px;
		display: grid;
		grid-template-columns: 48px minmax(0, 1fr) 48px;
		align-items: center;
	}
	.top .icon-btn {
		width: 48px;
		height: 48px;
	}
	.playon {
		min-width: 0;
		display: flex;
		justify-content: center;
	}
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		max-width: 100%;
		min-height: 44px;
		padding: 0 14px;
		border: 0;
		border-radius: 999px;
		background: color-mix(in srgb, var(--bg-raised) 60%, transparent);
		color: var(--fg);
		font: inherit;
		font-size: 0.875rem;
		font-weight: 550;
		cursor: pointer;
	}
	.chip:hover {
		background: color-mix(in srgb, var(--bg-raised) 85%, transparent);
	}
	.chip-text {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--fg-muted);
	}
	.chip.remote .dot {
		background: var(--accent);
	}
	/* "Play on…": a sheet across the bottom of the screen, above the tab bar. */
	.pop {
		position: fixed;
		left: 0.5rem;
		right: 0.5rem;
		bottom: calc(var(--tabs-h) + 0.5rem);
		z-index: 30;
		display: grid;
		justify-items: stretch;
	}
	.pop :global(.sheet) {
		width: auto;
		max-height: min(75vh, 640px);
	}

	/* The hero takes the height that is left; the cover is square, as big as fits. */
	.hero {
		position: relative;
		flex: 1 1 0;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}
	.stage {
		flex: 1 1 0;
		min-height: 0;
		width: 100%;
		display: grid;
		place-items: center;
		container-type: size;
	}
	.art-box {
		position: relative;
		width: min(100cqw, 100cqh);
		aspect-ratio: 1;
		border-radius: var(--radius);
		box-shadow: 0 24px 48px -24px rgb(0 0 0 / 50%);
	}
	.cover {
		position: relative;
		z-index: 1;
		border-radius: var(--radius);
	}
	.glowing {
		box-shadow: none;
	}
	.glowing .cover {
		transform: scale(var(--glow-cover, 1));
		box-shadow: 0 24px 48px -24px rgb(0 0 0 / 50%);
	}
	@media (prefers-reduced-motion: reduce) {
		.glowing .cover {
			transform: none;
		}
	}
	/* The visualiser draws over the cover. */
	.vis-over {
		position: absolute;
		inset: 0;
		z-index: 2;
	}
	.scroll {
		flex: 1 1 0;
		min-height: 0;
		overflow-y: auto;
		overscroll-behavior: contain;
		touch-action: pan-y pinch-zoom;
	}
	.toast {
		position: absolute;
		left: 50%;
		top: 40%;
		z-index: 4;
		transform: translate(-50%, -50%);
		padding: 0.5rem 1rem;
		border-radius: 999px;
		background: var(--bg-raised);
		color: var(--fg);
		box-shadow: var(--shadow);
		font-weight: 650;
		white-space: nowrap;
		pointer-events: none;
		opacity: 0;
		transition: opacity 0.2s;
	}
	.toast.shown {
		opacity: 1;
	}
	.toast:empty {
		padding: 0;
	}
	@media (prefers-reduced-motion: reduce) {
		.toast {
			transition: none;
		}
	}

	.meta {
		flex: none;
		min-height: 68px;
		display: grid;
		align-content: center;
		min-width: 0;
	}
	.title-row {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		height: 44px;
	}
	.title {
		flex: 1 1 auto;
		min-width: 0;
		margin: 0;
		font-size: 1.35rem;
		line-height: 1.2;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.rating {
		flex: none;
		margin-right: -6px;
	}
	.sub {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		height: 24px;
		margin: 0;
		font-size: 0.92rem;
	}
	p.sub {
		display: block;
		height: auto;
	}
	.who {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.who a:hover,
	.link {
		text-decoration: underline;
	}
	.badge {
		position: relative;
		flex: none;
		display: inline-flex;
		align-items: center;
		height: 24px;
		padding: 0 0.5rem;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: color-mix(in srgb, var(--bg-raised) 50%, transparent);
		color: var(--fg);
		font-family: var(--mono);
		font-size: 0.7rem;
		cursor: pointer;
	}
	/* A 44 px tall target, beyond the line's height. */
	.badge::after {
		content: '';
		position: absolute;
		inset: -10px -4px;
	}
	.badge.hires {
		border-color: var(--accent);
	}

	.seek {
		flex: none;
		min-height: 56px;
	}
	.times {
		display: flex;
		justify-content: space-between;
		height: 16px;
		font-size: 0.75rem;
		line-height: 16px;
		font-family: var(--mono);
		font-variant-numeric: tabular-nums;
	}

	.transport {
		flex: none;
		height: 76px;
		display: grid;
		grid-template-columns: repeat(5, minmax(0, 1fr));
		align-items: center;
		justify-items: center;
	}
	.transport .icon-btn {
		width: 48px;
		height: 48px;
	}
	.transport .big {
		width: 52px;
		height: 52px;
	}
	.transport .play {
		width: 68px;
		height: 68px;
		background: var(--accent);
		color: var(--accent-fg);
		box-shadow: var(--shadow);
	}
	.transport .play:hover {
		background: var(--accent);
		filter: brightness(1.07);
	}
	.on {
		position: relative;
		color: var(--accent);
	}
	.one {
		position: absolute;
		right: 8px;
		bottom: 8px;
		font-size: 0.6rem;
		font-weight: 800;
	}

	.next {
		flex: none;
		max-height: 56px;
	}
	.next :global(.upnext) {
		max-width: none;
		margin-top: 0;
	}

	.actions {
		flex: none;
		min-height: 56px;
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: minmax(0, 1fr);
		align-items: center;
	}

	.rows {
		display: grid;
		gap: 2px;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 14px;
		width: 100%;
		min-height: 52px;
		padding: 8px 6px;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: var(--fg);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.row:hover {
		background: var(--bg-sunken);
	}
	.row:disabled {
		opacity: 0.45;
		cursor: default;
		background: transparent;
	}
	.row :global(svg) {
		flex: none;
		color: var(--fg-muted);
	}
	.row small {
		display: block;
		color: var(--fg-muted);
		font-size: 0.82rem;
	}
	.tick {
		width: 1.2em;
		color: var(--accent);
		font-weight: 700;
	}

	/* Shorter screens: Up next gives its room to the cover. */
	@media (max-height: 600px) {
		.next {
			display: none;
		}
	}
	/* Very short (a phone on its side): the column scrolls inside itself rather than squash
	   the controls, with the cover kept a usable size. */
	@media (max-height: 460px) {
		.np-phone {
			overflow-y: auto;
		}
		.hero {
			flex: none;
			height: 220px;
		}
	}
</style>
