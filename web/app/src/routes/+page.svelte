<script lang="ts">
	// Now playing: large art, track details, full transport.
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
	import { DEVICE_BUTTONS_NOTE } from '$lib/player/volume';
	import Ambient from '$lib/components/Ambient.svelte';
	import SleepSheet from '$lib/components/SleepSheet.svelte';
	import SignalPath from '$lib/components/SignalPath.svelte';
	import UpNext from '$lib/components/UpNext.svelte';
	import { badgeText, quality } from '$lib/lms/quality';
	import { formatLeft, sleepLeft } from '$lib/lms/sleep';
	import { reducedMotion } from '$lib/theme/motion';
	import { fade } from 'svelte/transition';

	let showMore = $state(false);
	let showSleep = $state(false);
	let showPath = $state(false);
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
	const shuffle = $derived(Number(app.now?.['playlist shuffle'] ?? 0));
	const repeat = $derived(Number(app.now?.['playlist repeat'] ?? 0));
	const vol = $derived(volume ?? app.now?.['mixer volume'] ?? 0);
	const badge = $derived(quality(track));
	const sleepIn = $derived(sleepLeft(app.now));
	/** Art and title cross-fade between tracks (not under "reduce motion"). */
	const trackKey = $derived(`${track?.id ?? ''}|${title}`);
	const swap = { duration: reducedMotion() ? 0 : 350 };
	const pos = $derived(
		app.now?.playlist_tracks ? `${Number(app.now.playlist_cur_index ?? 0) + 1} of ${app.now.playlist_tracks}` : '',
	);

	function setVolume(v: number) {
		volume = v;
		clearTimeout(volTimer);
		volTimer = setTimeout(async () => {
			await app.send(cmd.volume(v));
			volume = null;
		}, 150);
	}
</script>

<section class="np page">
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
				disabled={!app.current}
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
		<a class="btn act" href="#/queue" aria-label="Queue"><Icon name="queue" size={18} /><span class="label">Queue</span></a>
		{#if typeof track?.id === 'number' && track.id > 0}
			<button class="btn act" onclick={() => (showMore = true)} aria-label="More"><Icon name="more" size={18} /><span class="label">More</span></button>
		{/if}
	</div>

	<div class="art-wrap">
		{#key trackKey}
			<div class="art-box" in:fade={swap}>
				<Art path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 800)} alt={title ? `Cover of ${textOf(track?.album)}` : ''} label={title} rounded="var(--radius)" eager />
			</div>
		{/key}
	</div>

	<div class="info">

		{#if !app.current && app.playerId && app.playerId === app.deviceMac}
			<h1>Getting this device ready…</h1>
			<p class="muted">Music will play here. Pick something to play, or choose another player with the Play on… button.</p>
		{:else if !app.current}
			<h1>Nothing to control yet</h1>
			<p class="muted">Choose a player with the Play on… button by the volume.</p>
		{:else if !track && !app.now?.current_title}
			<h1>Nothing playing</h1>
			<p class="muted">Pick an album to start. <a class="link" href="#/albums">Browse albums</a></p>
		{:else}
			{#key trackKey}<h1 class="title" in:fade={swap}>{title}</h1>{/key}
			<p class="artist">
				{#if artistHref}<a href={artistHref}>{textOf(track?.artist)}</a>{:else}{textOf(track?.artist)}{/if}
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
		{/if}

		<div class="progress">
			<SeekBar
				trackId={typeof track?.id === 'number' && track.id > 0 && !isRemote(track.remote) ? track.id : null}
				value={shown}
				{total}
				disabled={!total || app.now?.can_seek === 0}
				oninput={(v) => (seeking = v)}
				onchange={async (v) => {
					await app.seek(v);
					seeking = null;
				}}
			/>
			<div class="times muted"><span>{duration(shown)}</span><span>-{duration(Math.max(0, total - shown))}</span></div>
		</div>

		<div class="transport">
			<button class="icon-btn" class:on={shuffle > 0} aria-pressed={shuffle > 0} aria-label="Shuffle" onclick={() => app.send(cmd.shuffle(shuffle ? 0 : 1))} disabled={!app.now}>
				<Icon name="shuffle" />
			</button>
			<button class="icon-btn big" aria-label="Previous" onclick={() => app.send(cmd.skip(-1))} disabled={!app.now}><Icon name="prev" size={30} /></button>
			<button class="icon-btn play" aria-label={app.playing ? 'Pause' : 'Play'} onclick={() => app.send(app.playing ? cmd.pause(true) : cmd.play())} disabled={!app.now}>
				<Icon name={app.playing ? 'pause' : 'play'} size={34} />
			</button>
			<button class="icon-btn big" aria-label="Next" onclick={() => app.send(cmd.skip(1))} disabled={!app.now}><Icon name="next" size={30} /></button>
			<button
				class="icon-btn"
				class:on={repeat > 0}
				aria-pressed={repeat > 0}
				aria-label={repeat === 1 ? 'Repeat: this track' : repeat === 2 ? 'Repeat: all' : 'Repeat: off'}
				onclick={() => app.send(cmd.repeat(((repeat + 1) % 3) as 0 | 1 | 2))}
				disabled={!app.now}
			>
				<Icon name="repeat" />{#if repeat === 1}<span class="one">1</span>{/if}
			</button>
		</div>

		<UpNext />
		{#if sleepError}<p class="muted" role="alert">{sleepError}</p>{/if}

		<div class="volrow">
			{#if device.buttonsOnly(app.playerId)}
				<p class="volume muted" data-volume-note><Icon name="volume" size={20} /><span>{DEVICE_BUTTONS_NOTE}</span></p>
			{:else}
				<label class="volume">
					<Icon name={vol > 0 ? 'volume' : 'mute'} size={20} />
					<span class="sr-only">Volume</span>
					<input type="range" min="0" max="100" value={vol} oninput={(e) => setVolume(Number(e.currentTarget.value))} disabled={!app.now} />
					<span class="muted vnum">{Math.round(vol)}</span>
				</label>
			{/if}
			<OutputButton />
		</div>

		{#if showMore && typeof track?.id === 'number'}<ContextMenu kind="track" id={track.id} title={title} onclose={() => (showMore = false)} />{/if}
		{#if showSleep}<SleepSheet onclose={() => (showSleep = false)} />{/if}
		{#if showPath}<SignalPath onclose={() => (showPath = false)} />{/if}
	</div>
</section>

<style>
	/* Wide screens: cover on the left; player, actions and details on the right. */
	.np {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		grid-template-rows: 1fr auto auto 1fr;
		grid-template-areas:
			'art .'
			'art bar'
			'art info'
			'art .';
		column-gap: clamp(1.5rem, 4vw, 4rem);
		min-height: calc(var(--app-h) - var(--bar-h) - 3rem);
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
		width: 32px;
		height: 32px;
		margin-left: 0.15rem;
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
		grid-area: art;
		align-self: center;
		justify-self: end;
		width: 100%;
		max-width: 560px;
	}
	.art-box {
		box-shadow: 0 30px 60px -20px rgb(0 0 0 / 45%);
		border-radius: var(--radius);
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

	/* Phones: one column that fills the screen above the tab bar without scrolling. The
	   player and actions sit at the top; the cover takes the height the details and
	   controls leave (up to the full width), and everything below it is centred. */
	@media (max-width: 760px) {
		.np {
			display: flex;
			flex-direction: column;
			height: calc(var(--app-h) - var(--tabs-h));
			min-height: 0;
			padding: max(0.75rem, env(safe-area-inset-top)) 1.25rem 1rem;
			overflow: hidden;
		}
		.bar {
			flex: none;
			max-width: none;
			margin-bottom: 1rem;
		}
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
		.act.set {
			padding: 0.4em 0.75em;
			border-radius: 999px;
		}
		.art-wrap {
			flex: 1 1 0;
			min-height: 120px;
			max-width: none;
			display: grid;
			place-items: center;
			container-type: size;
		}
		.art-box {
			width: min(100cqw, 100cqh);
			box-shadow: 0 24px 48px -24px rgb(0 0 0 / 50%);
		}
		.info {
			flex: none;
			max-width: none;
			margin-top: 1.25rem;
			gap: 0.3rem;
			text-align: center;
		}
		.title {
			font-size: clamp(1.35rem, 6vw, 1.75rem);
			display: -webkit-box;
			-webkit-line-clamp: 2;
			line-clamp: 2;
			-webkit-box-orient: vertical;
			overflow: hidden;
		}
		.artist,
		.album,
		.details {
			white-space: nowrap;
			overflow: hidden;
			text-overflow: ellipsis;
		}
		.progress {
			margin-top: 0.9rem;
		}
		.transport {
			max-width: 400px;
			width: 100%;
			justify-self: center;
			margin: 0.35rem 0 0.5rem;
		}
		.volrow {
			max-width: 400px;
			width: 100%;
			justify-self: center;
		}
	}
	/* Short phone screens: drop the quality line and tighten a little before the cover
	   gets too small. */
	@media (max-width: 760px) and (max-height: 700px) {
		.details {
			display: none;
		}
		.info {
			margin-top: 0.9rem;
		}
		.progress {
			margin-top: 0.6rem;
		}
	}
	/* Very short screens (landscape phones): let the page scroll rather than squash. */
	@media (max-width: 760px) and (max-height: 520px) {
		.np {
			height: auto;
			overflow: visible;
		}
		.art-wrap {
			flex: none;
			height: 50vh;
		}
	}
</style>
