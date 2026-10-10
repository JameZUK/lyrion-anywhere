<script lang="ts">
	// Lock-screen / notification metadata and hardware media keys (Media Session API),
	// controlling the selected player. While downloads play on this device, the offline
	// player has the lock screen instead.
	import { app, api } from '$lib/state/app.svelte';
	import { offlinePlayer } from '$lib/offline/index.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';

	const ms = typeof navigator !== 'undefined' && 'mediaSession' in navigator ? navigator.mediaSession : null;
	const track = $derived(app.now?.playlist_loop?.[0]);

	const mine = $derived(!offlinePlayer.active);

	$effect(() => {
		if (!ms || !mine) return;
		const actions: [MediaSessionAction, () => void][] = [
			['play', () => app.send(cmd.play())],
			['pause', () => app.send(cmd.pause(true))],
			['nexttrack', () => app.send(cmd.skip(1))],
			['previoustrack', () => app.send(cmd.skip(-1))],
		];
		for (const [a, fn] of actions) {
			try {
				ms.setActionHandler(a, fn);
			} catch {
				/* unsupported action */
			}
		}
		ms.setActionHandler('seekto', (d) => {
			if (typeof d.seekTime === 'number') void app.seek(d.seekTime);
		});
		return () => {
			// The offline player may have taken the controls over already: leave its handlers.
			if (!offlinePlayer.active) for (const [a] of actions) ms.setActionHandler(a, null);
		};
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
		if (path) api.art(path).then((src) => set([{ src, sizes: '512x512' }]), () => {});
		return () => (live = false);
	});

	$effect(() => {
		if (ms && mine) ms.playbackState = app.now?.mode === 'play' ? 'playing' : app.now?.mode === 'pause' ? 'paused' : 'none';
	});
</script>
