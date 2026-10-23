<script lang="ts">
	// Lock-screen / notification metadata and hardware media keys (Media Session API),
	// controlling the selected player. While downloads play on this device, the offline
	// player has the lock screen instead.
	import { app, api } from '#lib/state/app.svelte';
	import { offlinePlayer } from '#lib/offline/index.svelte';
	import * as cmd from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import { controls } from '#lib/player/controls.svelte';

	const ms = typeof navigator !== 'undefined' && 'mediaSession' in navigator ? navigator.mediaSession : null;
	const track = $derived(app.now?.playlist_loop?.[0]);

	const mine = $derived(!offlinePlayer.active);

	/** Seconds moved by the system's seek-backward and seek-forward buttons, unless it says. */
	const SEEK_OFFSET = 10;
	const total = $derived(Number(app.now?.duration ?? track?.duration ?? 0));
	/** Which controls work now: out of reach, only what this device can do by itself. */
	const g = $derived(controls.gates);
	const seekable = $derived(total > 0 && app.now?.can_seek !== 0 && g.seek);

	function seekTo(seconds: number) {
		if (!Number.isFinite(seconds)) return;
		void controls.seek(Math.max(0, Math.min(Math.max(0, total - 1), seconds)));
	}

	// Every action that works now: play, pause, stop, previous and next, and, for a track with
	// a length, seeking (to a point, or back and forward). The system shows only these.
	$effect(() => {
		if (!ms || !mine) return;
		const canSeek = seekable;
		const { playPause, next, previous, other } = g;
		const actions: [MediaSessionAction, MediaSessionActionHandler | null][] = [
			['play', playPause ? () => controls.play() : null],
			['pause', playPause ? () => controls.pause() : null],
			['stop', other ? () => app.send(cmd.stop()) : null],
			['nexttrack', next ? () => controls.next() : null],
			['previoustrack', previous ? () => controls.previous() : null],
			['seekto', canSeek ? (d) => typeof d.seekTime === 'number' && seekTo(d.seekTime) : null],
			['seekbackward', canSeek ? (d) => seekTo(app.elapsed - (d.seekOffset || SEEK_OFFSET)) : null],
			['seekforward', canSeek ? (d) => seekTo(app.elapsed + (d.seekOffset || SEEK_OFFSET)) : null],
		];
		for (const [a, fn] of actions) {
			try {
				ms.setActionHandler(a, fn);
			} catch {
				/* unsupported action */
			}
		}
		return () => {
			// The offline player may have taken the controls over already: leave its handlers.
			if (offlinePlayer.active) return;
			for (const [a] of actions) {
				try {
					ms.setActionHandler(a, null);
				} catch {
					/* unsupported action */
				}
			}
		};
	});

	// Where the track is, for the system's own progress bar (updated with each status).
	$effect(() => {
		if (!ms || !mine || typeof ms.setPositionState !== 'function') return;
		const position = Number(app.now?.time ?? 0);
		try {
			if (seekable) ms.setPositionState({ duration: total, position: Math.max(0, Math.min(total, position)), playbackRate: 1 });
			else ms.setPositionState();
		} catch {
			/* a browser that refuses odd values */
		}
	});

	$effect(() => {
		if (!ms || !mine || typeof MediaMetadata === 'undefined') return;
		const title = textOf(track?.title ?? app.now?.current_title);
		const artist = textOf(track?.artist);
		const album = textOf(track?.album);
		const path = cmd.coverPath(track?.coverid ?? track?.artwork_track_id, 512);
		let live = true;
		const set = (artwork: MediaImage[]) => {
			if (live) ms.metadata = title ? new MediaMetadata({ title, artist, album, artwork }) : null;
		};
		set([]);
		// The system's media controls show this art: keep its blob: URL while it is current.
		const release = path ? api.retainArt?.(path) : undefined;
		if (path) api.art(path).then((src) => set([{ src, sizes: '512x512' }]), () => {});
		return () => {
			live = false;
			release?.();
		};
	});

	$effect(() => {
		if (ms && mine) ms.playbackState = app.now?.mode === 'play' ? 'playing' : app.now?.mode === 'pause' ? 'paused' : 'none';
	});
</script>
