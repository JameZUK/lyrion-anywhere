<script lang="ts">
	// Now playing: large art, track details, full transport.
	import { app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { duration, formatDetails, textOf } from '$lib/lms/format';
	import Art from '$lib/components/Art.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import PlayerSwitcher from '$lib/components/PlayerSwitcher.svelte';

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
	<div class="art-wrap">
		<Art path={cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 800)} alt={title ? `Cover of ${textOf(track?.album)}` : ''} label={title} rounded="var(--radius)" eager />
	</div>

	<div class="info">
		<button class="btn player" onclick={() => (switcher = !switcher)} aria-expanded={switcher}>
			<Icon name="speaker" size={18} />{textOf(app.current?.name) || 'Choose a player'}
			<Icon name="chevron" size={16} />
		</button>
		{#if switcher}<div class="pop"><PlayerSwitcher onclose={() => (switcher = false)} /></div>{/if}

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
			<input
				type="range"
				min="0"
				max={Math.max(total, 1)}
				value={shown}
				disabled={!total || app.now?.can_seek === 0}
				aria-label="Position"
				oninput={(e) => (seeking = Number(e.currentTarget.value))}
				onchange={async (e) => {
					await app.seek(Number(e.currentTarget.value));
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

		<label class="volume">
			<Icon name={vol > 0 ? 'volume' : 'mute'} size={20} />
			<span class="sr-only">Volume</span>
			<input type="range" min="0" max="100" value={vol} oninput={(e) => setVolume(Number(e.currentTarget.value))} disabled={!app.now} />
			<span class="muted vnum">{Math.round(vol)}</span>
		</label>

		<a class="btn queue-link" href="#/queue"><Icon name="queue" size={18} />Queue</a>
	</div>
</section>

<style>
	.np {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		gap: clamp(1.5rem, 4vw, 4rem);
		align-items: center;
		min-height: calc(100vh - var(--bar-h) - 3rem);
	}
	.art-wrap {
		max-width: 560px;
		width: 100%;
		justify-self: end;
		box-shadow: 0 30px 60px -20px rgb(0 0 0 / 45%);
		border-radius: var(--radius);
	}
	.info {
		position: relative;
		display: grid;
		gap: 0.4rem;
		max-width: 520px;
	}
	.player {
		justify-self: start;
		margin-bottom: 1rem;
	}
	.pop {
		position: absolute;
		top: 3rem;
		left: 0;
		z-index: 10;
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
	.vnum {
		width: 2.5ch;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.queue-link {
		justify-self: start;
		margin-top: 1rem;
	}
	@media (max-width: 760px) {
		.np {
			grid-template-columns: minmax(0, 1fr);
			gap: 1.25rem;
			min-height: 0;
		}
		.art-wrap {
			justify-self: center;
			max-width: min(100%, 420px);
		}
		.player {
			order: -1;
		}
	}
</style>
