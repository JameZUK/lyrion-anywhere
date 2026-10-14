<script lang="ts">
	// Now playing: large art, track details, full transport. Three layouts (Wave 2 §4.2): Art,
	// Lyrics and Info, from the segmented control or the keys 1, 2 and 3; in wide windows
	// (over 840 px) the art stays on the left with lyrics or info beside it, and the queue can
	// be pinned on the right. The visualiser (§4.1) takes the art's place while this browser
	// is the player and it is switched on. Phones and narrow windows have a layout of their
	// own (NowPlayingPhone), whose pager shows the same three layouts.
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, formatDetails, isRemote, textOf } from '$lib/lms/format';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import OutputButton from '$lib/components/OutputButton.svelte';
	import PlayingElsewhere from '$lib/components/PlayingElsewhere.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import SeekBar from '$lib/components/SeekBar.svelte';
	import { device } from '$lib/player/device.svelte';
	import { controls } from '$lib/player/controls.svelte';
	import { DEVICE_BUTTONS_NOTE } from '$lib/player/volume';
	import Ambient from '$lib/components/Ambient.svelte';
	import SleepSheet from '$lib/components/SleepSheet.svelte';
	import SignalPath from '$lib/components/SignalPath.svelte';
	import UpNext from '$lib/components/UpNext.svelte';
	import QueuePeek from '$lib/components/QueuePeek.svelte';
	import ShuffleButton from '$lib/components/ShuffleButton.svelte';
	import DeviceWait from '$lib/components/DeviceWait.svelte';
	import LyricsView from '$lib/components/LyricsView.svelte';
	import RatingControl from '$lib/components/RatingControl.svelte';
	import { lyrics } from '$lib/state/lyrics';
	import { lineAt } from '$lib/lyrics/lyrics';
	import { badgeText, quality } from '$lib/lms/quality';
	import { formatLeft, sleepLeft } from '$lib/lms/sleep';
	import { reducedMotion } from '$lib/theme/motion';
	import { fade } from 'svelte/transition';
	import LyricsLines from '$lib/components/LyricsLines.svelte';
	import NowPlayingInfo from '$lib/components/NowPlayingInfo.svelte';
	import QueueList from '$lib/components/QueueList.svelte';
	import Visualiser from '$lib/components/Visualiser.svelte';
	import GlowHalo from '$lib/components/GlowHalo.svelte';
	import { GLOW_REST, type GlowStyle } from '$lib/visual/glow';
	import { nowPlaying } from '$lib/nowplaying/store.svelte';
	import { LAYOUTS, LAYOUT_LABELS, layoutForKey, swipeLayout, type Layout } from '$lib/nowplaying/prefs';
	import { STYLE_LABELS, nextStyle } from '$lib/visual/draw';
	import { isThisDevice } from '$lib/player/outputs';
	import { haptics } from '$lib/haptics/haptics.svelte';
	import NowPlayingPhone from '$lib/components/NowPlayingPhone.svelte';

	let showMore = $state(false);
	let showSleep = $state(false);
	let showPath = $state(false);
	let showLyrics = $state(false);
	let showQueue = $state(false);
	/** A swipe up over the cover peeks at the queue (touch and pen). */
	let peekSwipe: { id: number; x: number; y: number } | null = null;
	function peekDown(e: PointerEvent) {
		if (e.pointerType === 'mouse') return;
		peekSwipe = { id: e.pointerId, x: e.clientX, y: e.clientY };
	}
	function peekUp(e: PointerEvent) {
		const s = peekSwipe;
		peekSwipe = null;
		if (!s || s.id !== e.pointerId) return;
		const dx = e.clientX - s.x;
		const dy = e.clientY - s.y;
		if (dy <= -SWIPE_UP_PX && Math.abs(dy) > Math.abs(dx) * 1.5 && app.current) showQueue = true;
		// Sideways: the next or previous layout. A swipe across the visualiser changes its
		// style instead (it handles that itself).
		else if (Math.abs(dx) >= SWIPE_SIDE_PX && Math.abs(dx) > Math.abs(dy) * 1.5 && !(e.target as Element | null)?.closest?.('[data-visualiser]')) {
			setLayout(swipeLayout(layout, dx));
		}
	}
	const SWIPE_UP_PX = 60;
	const SWIPE_SIDE_PX = 60;

	// --- layouts (§4.2) -----------------------------------------------------------------
	const layout = $derived(nowPlaying.layout);
	function setLayout(l: Layout) {
		if (l !== nowPlaying.layout) nowPlaying.setLayout(l);
	}
	/** Keys 1, 2 and 3 pick a layout (not while typing, with a modifier, or in a dialog). */
	function onkeydown(e: KeyboardEvent) {
		if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.repeat) return;
		const t = e.target as HTMLElement | null;
		if (t?.closest?.('input, textarea, select, [contenteditable=""], [contenteditable="true"], dialog, [role="dialog"], [role="menu"]')) return;
		const l = layoutForKey(e.key);
		if (!l) return;
		e.preventDefault();
		setLayout(l);
	}

	/** Wide windows: the art stays on the left, lyrics or info beside it. */
	let wide = $state(false);
	/** Room for the queue beside Now playing as well. */
	let roomy = $state(false);
	let reduce = $state(false);
	$effect(() => {
		const mq = (q: string) => (typeof matchMedia === 'function' ? matchMedia(q) : null);
		const w = mq('(min-width: 841px)');
		const r = mq('(min-width: 1200px)');
		const m = mq('(prefers-reduced-motion: reduce)');
		const update = () => {
			wide = !!w?.matches;
			roomy = !!r?.matches;
			reduce = !!m?.matches;
		};
		update();
		for (const q of [w, r, m]) q?.addEventListener?.('change', update);
		return () => {
			for (const q of [w, r, m]) q?.removeEventListener?.('change', update);
		};
	});
	/** What the art area shows: in narrow windows, the layout's own content takes its place. */
	const stage = $derived<Layout>(wide ? 'art' : layout);
	/** Wide windows: lyrics or info in the right-hand column. */
	const side = $derived<Layout | null>(wide && layout !== 'art' ? layout : null);
	const queueSide = $derived(roomy && nowPlaying.queuePanel && !!app.current);

	// --- the visualiser (§4.1) ----------------------------------------------------------
	/** This browser is the player (a remote player keeps the blurred-art background only). */
	const local = $derived(!!app.current && isThisDevice(app.playerId) && (device.state === 'connected' || device.state === 'reconnecting'));
	const visualising = $derived(local && nowPlaying.visualiser && !reduce && stage === 'art');
	/** Glow: a halo in the cover's colours behind it (still, at rest, under reduced motion). */
	const glowing = $derived(local && nowPlaying.visualiser && nowPlaying.style === 'glow' && stage === 'art');
	let section = $state<HTMLElement>();
	/** Each frame, the visualiser's beat pulse sizes the halo and (just) the cover. */
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
	let sleepError = $state('');

	let seeking = $state<number | null>(null);
	let volume = $state<number | null>(null);
	let volTimer: ReturnType<typeof setTimeout> | undefined;

	const track = $derived(app.now?.playlist_loop?.[0]);
	const total = $derived(app.now?.duration ?? track?.duration ?? 0);
	const shown = $derived(seeking ?? app.elapsed);
	const title = $derived(textOf(track?.title ?? app.now?.current_title));
	const artistHref = $derived(cmd.link('artist', track?.artist_id));
	const albumHref = $derived(cmd.link('album', track?.album_id));
	const repeat = $derived(Number(app.now?.['playlist repeat'] ?? 0));
	/** Which controls work now (all of them while the server is in reach). */
	const g = $derived(controls.gates);
	const vol = $derived(volume ?? app.now?.['mixer volume'] ?? 0);
	const badge = $derived(quality(track));
	const sleepIn = $derived(sleepLeft(app.now));
	/** Art and title cross-fade between tracks (not under "reduce motion"). */
	const trackKey = $derived(`${track?.id ?? ''}|${title}`);
	const swap = { duration: reducedMotion() ? 0 : 350 };
	/** Lyrics for library tracks (not streams). */
	const lyricsId = $derived(typeof track?.id === 'number' && track.id > 0 && !isRemote(track.remote) ? track.id : null);
	$effect(() => {
		void lyrics.load(lyricsId);
	});
	const lyricLine = $derived(lyrics.peek && lyrics.lyrics?.synced ? lyrics.lyrics.lines[lineAt(lyrics.lyrics, app.elapsed * 1000)]?.text ?? '' : '');
	const pos = $derived(
		app.now?.playlist_tracks ? `${Number(app.now.playlist_cur_index ?? 0) + 1} of ${app.now.playlist_tracks}` : '',
	);

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

<svelte:window {onkeydown} />

{#if !wide}
	<NowPlayingPhone {reduce} />
{:else}
<section class="np page" class:wide class:has-side={!!side} class:has-queue={queueSide} bind:this={section} data-layout={layout}>
	<Ambient path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 120)} />
	<div class="bar">
		{#if app.remote || app.moving}<div class="where"><PlayingElsewhere /></div>{/if}
		<span class="sleepgroup">
			<button
				class="btn act"
				class:set={sleepIn !== null}
				onclick={() => (showSleep = true)}
				aria-haspopup="dialog"
				aria-label={sleepIn !== null ? `Sleep timer: stops in ${formatLeft(sleepIn)}` : 'Sleep timer'}
				disabled={!app.current || !g.other}
				data-sleep-button
			>
				<Icon name="moon" size={18} /><span class="label">{sleepIn !== null ? formatLeft(sleepIn) : 'Sleep'}</span>
			</button>
			{#if sleepIn !== null}
				<button class="icon-btn cancel" aria-label="Turn the sleep timer off" onclick={async () => (sleepError = await app.cancelSleep())} data-sleep-cancel>
					<Icon name="close" size={16} />
				</button>
			{/if}
		</span>
		{#if lyrics.lyrics && lyricsId !== null}
			<button class="btn act" onclick={() => (showLyrics = true)} aria-haspopup="dialog" aria-label="Lyrics" data-lyrics-button><Icon name="lyrics" size={18} /><span class="label">Lyrics</span></button>
		{/if}
		{#if app.current}<a class="btn act" href="#/car" aria-label="Car mode" data-car-link><Icon name="car" size={18} /><span class="label">Car</span></a>{/if}
		{#if app.current}<a class="btn act" href="#/alarms" aria-label="Alarms" data-alarms-link><Icon name="alarm" size={18} /><span class="label">Alarms</span></a>{/if}
		{#if roomy && app.current}
			<!-- With room for it, Queue pins or hides the queue beside Now playing. -->
			<button
				class="btn act"
				class:set={nowPlaying.queuePanel}
				aria-pressed={nowPlaying.queuePanel}
				aria-label="Show the queue beside Now playing"
				onclick={() => nowPlaying.setQueuePanel(!nowPlaying.queuePanel)}
				data-queue-panel-toggle
			><Icon name="queue" size={18} /><span class="label">Queue</span></button>
		{:else}
			<button class="btn act" onclick={() => (showQueue = true)} aria-haspopup="dialog" aria-label="Queue" disabled={!app.current} data-queue-peek><Icon name="queue" size={18} /><span class="label">Queue</span></button>
		{/if}
		{#if typeof track?.id === 'number' && track.id > 0}
			<!-- The track's menu is LMS's own: it needs the server. -->
			<button class="btn act" onclick={() => (showMore = true)} aria-label="More" {...controls.needsServer()} data-np-more><Icon name="more" size={18} /><span class="label">More</span></button>
		{/if}
	</div>

	<div class="modes">
		<div class="seg" role="group" aria-label="Now playing layout" data-layouts>
			{#each LAYOUTS as l, i (l)}
				<button
					class="segbtn"
					aria-pressed={layout === l}
					aria-keyshortcuts={String(i + 1)}
					onclick={() => setLayout(l)}
					data-layout-button={l}
				><Icon name={l === 'art' ? 'art' : l === 'lyrics' ? 'lyrics' : 'info'} size={16} /><span>{LAYOUT_LABELS[l]}</span></button>
			{/each}
		</div>
		{#if local}
			<button
				class="icon-btn mode"
				class:on={nowPlaying.visualiser && !reduce}
				aria-pressed={nowPlaying.visualiser && !reduce}
				aria-label={reduce ? 'Visualiser (off while your device asks for reduced motion)' : 'Visualiser'}
				disabled={reduce}
				onclick={() => nowPlaying.setVisualiser(!nowPlaying.visualiser)}
				data-visualiser-toggle
			><Icon name="visualiser" size={20} /></button>
			{#if nowPlaying.visualiser && !reduce}
				<button
					class="btn mode style"
					aria-label={`Visualiser style: ${STYLE_LABELS[nowPlaying.style]}. Change style`}
					onclick={() => nowPlaying.setStyle(nextStyle(nowPlaying.style))}
					data-visualiser-style
				>{STYLE_LABELS[nowPlaying.style]}</button>
			{/if}
		{/if}
	</div>

	<div
		class="art-wrap"
		class:pane-stage={stage !== 'art'}
		onpointerdown={peekDown}
		onpointerup={peekUp}
		onpointercancel={() => (peekSwipe = null)}
		role="presentation"
		data-peek-area
	>
		{#if stage === 'art'}
			{#if visualising && nowPlaying.style !== 'glow'}
				<div class="art-box vis-box">
					<Visualiser style={nowPlaying.style} onstyle={(s) => nowPlaying.setStyle(s)} />
				</div>
			{:else}
				{#key trackKey}
					<div class="art-box" class:glowing in:fade={swap}>
						{#if glowing}<GlowHalo path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 120)} playing={app.playing} still={reduce} />{/if}
						<div class="cover">
							<Art path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 800)} alt={title ? `Cover of ${textOf(track?.album)}` : ''} label={title} rounded="var(--radius)" eager />
						</div>
						{#if visualising}<Visualiser style="glow" onstyle={(s) => nowPlaying.setStyle(s)} {onglow} />{/if}
					</div>
				{/key}
			{/if}
		{:else if stage === 'lyrics'}
			<div class="pane" aria-label="Lyrics" role="region" data-lyrics-pane><LyricsLines compact /></div>
		{:else}
			<div class="pane" aria-label="Track information" role="region"><NowPlayingInfo /></div>
		{/if}
	</div>

	{#if side}
		<div class="side-pane pane" aria-label={side === 'lyrics' ? 'Lyrics' : 'Track information'} role="region" data-side-pane={side}>
			{#if side === 'lyrics'}<LyricsLines compact />{:else}<NowPlayingInfo />{/if}
		</div>
	{/if}

	{#if queueSide}
		<aside class="queue-side" aria-label="Queue" data-queue-side><h2 class="qs-title">Queue</h2><QueueList side /></aside>
	{/if}

	<div class="info">

		{#if !app.current && app.playerId && app.playerId === app.deviceMac}
			<DeviceWait />
		{:else if !app.current}
			<h1>Nothing to control yet</h1>
			<p class="muted">Choose a player with the Play on… button by the volume.</p>
		{:else if !track && !app.now?.current_title}
			<h1>Nothing playing</h1>
			<p class="muted">Pick an album to start. <a class="link" href="#/albums">Browse albums</a></p>
		{:else}
			{#key trackKey}<h1 class="title" in:fade={swap}>{title}</h1>{/key}
			<p class="artist">
				{#if artistHref}<a href={artistHref}>{textOf(track?.artist)}</a>{:else}{textOf(track?.artist)}{/if}{#if track?.album}<span class="album-inline muted">{` · ${textOf(track.album)}`}</span>{/if}
			</p>
			<p class="album muted">
				{#if albumHref}<a href={albumHref}>{textOf(track?.album)}</a>{:else}{textOf(track?.album)}{/if}
				{#if track?.year}· {textOf(track.year)}{/if}
			</p>
			<p class="details muted">
				{#if badge}
					<button class="badge" class:hires={badge.hiRes} onclick={() => (showPath = true)} aria-haspopup="dialog" aria-label={`${badgeText(badge)}. Show the signal path`} data-quality-badge>{badgeText(badge)}</button>
				{:else}{formatDetails(track ?? {})}{/if}{pos ? ` · ${pos}` : ''}
			</p>
			{#if lyricsId !== null && layout !== 'info'}<div class="rating"><RatingControl track={lyricsId} /></div>{/if}
			{#if lyricLine && layout !== 'lyrics'}
				<button class="peek" onclick={() => (showLyrics = true)} aria-label={`Lyrics: ${lyricLine}. Show all the lyrics`} data-lyrics-peek>{lyricLine}</button>
			{/if}
		{/if}

		<div class="progress">
			<SeekBar
				trackId={typeof track?.id === 'number' && track.id > 0 && !isRemote(track.remote) ? track.id : null}
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

		{#if layout !== 'info'}<UpNext />{/if}
		{#if sleepError}<p class="muted" role="alert">{sleepError}</p>{/if}

		<div class="volrow">
			{#if device.buttonsOnly(app.playerId)}
				<p class="volume muted" data-volume-note><Icon name="volume" size={20} /><span>{DEVICE_BUTTONS_NOTE}</span></p>
			{:else}
				<label class="volume">
					<Icon name={vol > 0 ? 'volume' : 'mute'} size={20} />
					<span class="sr-only">Volume</span>
					<input type="range" min="0" max="100" value={vol} oninput={(e) => setVolume(Number(e.currentTarget.value))} disabled={!g.volume} />
					<span class="muted vnum">{Math.round(vol)}</span>
				</label>
			{/if}
			<OutputButton />
		</div>

		{#if showMore && typeof track?.id === 'number'}<ContextMenu kind="track" id={track.id} title={title} onclose={() => (showMore = false)} />{/if}
		{#if showSleep}<SleepSheet onclose={() => (showSleep = false)} />{/if}
		{#if showPath}<SignalPath onclose={() => (showPath = false)} />{/if}
		{#if showQueue}<QueuePeek onclose={() => (showQueue = false)} />{/if}
		{#if showLyrics}<LyricsView {title} onclose={() => (showLyrics = false)} />{/if}
	</div>
</section>
{/if}

<style>
	/* Wide screens: cover on the left; player, actions and details on the right. */
	.np {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		grid-template-rows: 1fr auto auto auto 1fr;
		grid-template-areas:
			'art .'
			'art bar'
			'art modes'
			'art info'
			'art .';
		column-gap: clamp(1.5rem, 4vw, 4rem);
		min-height: calc(var(--app-h) - var(--bar-h) - 3rem);
	}
	/* Wide windows, Lyrics or Info: the art stays left, the pane fills the right column. */
	.np.has-side {
		grid-template-rows: auto auto minmax(0, 1fr) auto;
		grid-template-areas:
			'art bar'
			'art modes'
			'art side'
			'art info';
		height: calc(var(--app-h) - var(--bar-h) - 3rem);
		min-height: 420px;
	}
	/* Room for the queue too: pinned on the right. */
	.np.has-queue {
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) minmax(260px, 320px);
		grid-template-areas:
			'art . queue'
			'art bar queue'
			'art modes queue'
			'art info queue'
			'art . queue';
	}
	.np.has-side.has-queue {
		grid-template-areas:
			'art bar queue'
			'art modes queue'
			'art side queue'
			'art info queue';
	}
	.modes {
		grid-area: modes;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.4rem;
		max-width: 520px;
		margin-bottom: 0.9rem;
	}
	.seg {
		display: inline-flex;
		padding: 2px;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: color-mix(in srgb, var(--bg-raised) 55%, transparent);
	}
	.segbtn {
		display: inline-flex;
		align-items: center;
		gap: 0.35rem;
		min-height: 44px;
		padding: 0 0.9rem;
		border: 0;
		border-radius: 999px;
		background: transparent;
		color: var(--fg-muted);
		font: inherit;
		font-size: 0.88rem;
		font-weight: 600;
	}
	.segbtn[aria-pressed='true'] {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.mode.on {
		color: var(--accent);
	}
	.style {
		font-size: 0.85rem;
	}
	.side-pane {
		grid-area: side;
		max-width: 520px;
		margin-bottom: 1rem;
	}
	.pane {
		min-height: 0;
		overflow-y: auto;
		/* Sideways swipes change the layout; up and down scroll. */
		touch-action: pan-y pinch-zoom;
		overscroll-behavior: contain;
	}
	.art-wrap.pane-stage {
		align-self: stretch;
		display: flex;
		flex-direction: column;
		max-height: calc(var(--app-h) - var(--bar-h) - 3rem);
	}
	.art-wrap.pane-stage .pane {
		flex: 1;
	}
	.queue-side {
		grid-area: queue;
		align-self: stretch;
		min-height: 0;
		max-height: calc(var(--app-h) - var(--bar-h) - 3rem);
		overflow-y: auto;
		padding: 0 0 0 1rem;
		border-left: 1px solid var(--line);
	}
	.qs-title {
		margin: 0.25rem 0 0;
		font-size: 1rem;
	}
	.bar {
		grid-area: bar;
		position: relative;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: flex-end;
		gap: 0.5rem;
		max-width: 520px;
		margin-bottom: 1.25rem;
		container-type: inline-size;
	}
	/* The "Playing on …" strip takes the room left by the action buttons, which give up
	   their labels first when the column is narrow (e.g. iPad portrait). */
	.where {
		flex: 1 1 100%;
		min-width: 0;
	}
	.act {
		flex: none;
	}
	.sleepgroup {
		display: inline-flex;
		align-items: center;
		flex: none;
	}
	.act.set {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		font-variant-numeric: tabular-nums;
	}
	.cancel {
		width: 44px;
		height: 44px;
	}
	.badge {
		display: inline-flex;
		align-items: center;
		padding: 0.1rem 0.55rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: transparent;
		color: var(--fg-muted);
		font-size: 0.78rem;
		font-weight: 650;
		letter-spacing: 0.02em;
		vertical-align: baseline;
	}
	.badge.hires {
		border-color: var(--accent);
		color: var(--fg);
	}
	.rating {
		margin-top: 0.35rem;
	}
	/* The current lyric line, small, over the ambient background. */
	.peek {
		display: block;
		max-width: 100%;
		margin: 0.35rem 0 0;
		padding: 0.2rem 0;
		background: none;
		border: 0;
		color: var(--fg);
		font: inherit;
		font-size: 0.95rem;
		font-style: italic;
		text-align: inherit;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		cursor: pointer;
	}
	.badge:hover {
		background: color-mix(in srgb, var(--fg) 8%, transparent);
	}
	@container (max-width: 400px) {
		.act:not(.set) {
			padding: 0.5em;
			border-radius: 50%;
		}
		.act:not(.set) .label {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0 0 0 0);
		}
	}
	.art-wrap {
		/* Swiping up over the cover peeks at the queue; sideways changes the layout. */
		touch-action: pinch-zoom;
		grid-area: art;
		align-self: center;
		justify-self: end;
		width: 100%;
		max-width: 560px;
	}
	.art-box {
		position: relative;
		box-shadow: 0 30px 60px -20px rgb(0 0 0 / 45%);
		border-radius: var(--radius);
	}
	/* Glow: the halo (GlowHalo) sits behind the cover, which kicks very slightly on a beat. */
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
		box-shadow: 0 30px 60px -20px rgb(0 0 0 / 45%);
	}
	@media (prefers-reduced-motion: reduce) {
		.glowing .cover {
			transform: none;
		}
	}
	.info {
		grid-area: info;
		display: grid;
		gap: 0.4rem;
		max-width: 520px;
	}
	.title {
		font-size: clamp(1.6rem, 3.2vw, 2.4rem);
		margin: 0;
	}
	.artist {
		font-size: 1.15rem;
		font-weight: 550;
		margin: 0;
	}
	.album,
	.details {
		margin: 0;
	}
	/* The album on the artist line: only on short phone screens (where the album line goes). */
	.album-inline {
		display: none;
		font-weight: 400;
	}
	.details {
		font-size: 0.85rem;
	}
	.artist a:hover,
	.album a:hover,
	.link {
		text-decoration: underline;
	}
	.progress {
		margin-top: 1.25rem;
	}
	.times {
		display: flex;
		justify-content: space-between;
		font-size: 0.8rem;
		font-variant-numeric: tabular-nums;
	}
	.transport {
		display: flex;
		align-items: center;
		justify-content: space-between;
		max-width: 360px;
		margin: 0.5rem 0;
	}
	.big {
		width: 52px;
		height: 52px;
	}
	.play {
		width: 68px;
		height: 68px;
		background: var(--accent);
		color: var(--accent-fg);
	}
	.play:hover {
		background: var(--accent);
		filter: brightness(1.07);
	}
	.on {
		color: var(--accent);
		position: relative;
	}
	.one {
		position: absolute;
		font-size: 0.6rem;
		font-weight: 800;
		right: 6px;
		bottom: 6px;
	}
	.volrow {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		max-width: 360px;
	}
	.volume {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.6rem;
		color: var(--fg-muted);
	}
	p.volume {
		margin: 0;
		font-size: 0.9rem;
	}
	.vnum {
		width: 2.5ch;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
</style>
