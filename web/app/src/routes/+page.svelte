<script lang="ts">
	// Now playing: large art, track details, full transport.
	import { app } from '$lib/state/app.svelte';
	import { playerLabel } from '$lib/lms/sync';
	import * as cmd from '$lib/lms/commands';
	import { duration, formatDetails, isRemote, textOf } from '$lib/lms/format';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import PlayerSwitcher from '$lib/components/PlayerSwitcher.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import SeekBar from '$lib/components/SeekBar.svelte';
	import { device } from '$lib/player/device.svelte';
	import { DEVICE_BUTTONS_NOTE } from '$lib/player/volume';

	let showMore = $state(false);

	let seeking = $state<number | null>(null);
	let switcher = $state(false);
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
	<div class="bar">
		<button class="btn player" onclick={() => (switcher = !switcher)} aria-expanded={switcher}>
			<Icon name="speaker" size={18} /><span class="ellipsis">{playerLabel(app.playerId, app.now, app.players) || 'Choose a player'}</span>
			<Icon name="chevron" size={16} />
		</button>
		{#if switcher}<div class="pop"><PlayerSwitcher onclose={() => (switcher = false)} /></div>{/if}
		<span class="grow"></span>
		<a class="btn act" href="#/queue" aria-label="Queue"><Icon name="queue" size={18} /><span class="label">Queue</span></a>
		{#if typeof track?.id === 'number' && track.id > 0}
			<button class="btn act" onclick={() => (showMore = true)} aria-label="More"><Icon name="more" size={18} /><span class="label">More</span></button>
		{/if}
	</div>

	<div class="art-wrap">
		<div class="art-box">
			<Art path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 800)} alt={title ? `Cover of ${textOf(track?.album)}` : ''} label={title} rounded="var(--radius)" eager />
		</div>
	</div>

	<div class="info">

		{#if !app.current}
			<h1>Nothing to control yet</h1>
			<p class="muted">Choose a player above, or play on this device.</p>
		{:else if !track && !app.now?.current_title}
			<h1>Nothing playing</h1>
			<p class="muted">Pick an album to start. <a class="link" href="#/albums">Browse albums</a></p>
		{:else}
			<h1 class="title">{title}</h1>
			<p class="artist">
				{#if artistHref}<a href={artistHref}>{textOf(track?.artist)}</a>{:else}{textOf(track?.artist)}{/if}
			</p>
			<p class="album muted">
				{#if albumHref}<a href={albumHref}>{textOf(track?.album)}</a>{:else}{textOf(track?.album)}{/if}
				{#if track?.year}· {textOf(track.year)}{/if}
			</p>
			<p class="details muted">{formatDetails(track ?? {})}{pos ? ` · ${pos}` : ''}</p>
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

		{#if showMore && typeof track?.id === 'number'}<ContextMenu kind="track" id={track.id} title={title} onclose={() => (showMore = false)} />{/if}
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
		align-items: center;
		gap: 0.5rem;
		max-width: 520px;
		margin-bottom: 1.25rem;
	}
	.grow {
		flex: 1;
	}
	.player {
		min-width: 0;
		max-width: 100%;
	}
	.pop {
		position: absolute;
		top: 3rem;
		left: 0;
		z-index: 10;
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
	.volume {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		max-width: 360px;
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
		.act {
			padding: 0.5em;
			border-radius: 50%;
		}
		.act .label {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip: rect(0 0 0 0);
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
		.volume {
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
