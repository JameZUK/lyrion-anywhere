// What is playing, as the main screens show it (Wave 3.5 item 14: two engines, one face). While
// this device plays its downloads (the offline player's queue is active), the mini player, Now
// playing and the Queue show that engine's track, position and queue; otherwise the selected
// LMS player's. The transport goes the same way through `controls` (./controls.svelte).
import { app } from '$lib/state/app.svelte';
import { offlinePlayer } from '$lib/offline/index.svelte';
import { isRemote } from '$lib/lms/format';
import type { Track } from '$lib/lms/types';
import type { StoredTrack } from '$lib/offline/types';

/** A downloaded track as LMS would describe it (enough for the screens that show tracks). */
export function trackOf(t: StoredTrack | undefined): Track | undefined {
	if (!t) return undefined;
	return { id: t.id, title: t.title, artist: t.artist, album: t.album, ...(t.albumId ? { album_id: t.albumId } : {}), duration: t.duration };
}

class NowView {
	/** This device plays its downloads. */
	get downloads(): boolean {
		return offlinePlayer.active;
	}

	/** The track playing (or paused on), or none. */
	get track(): Track | undefined {
		return this.downloads ? trackOf(offlinePlayer.current) : app.now?.playlist_loop?.[0];
	}

	/** Seconds into it. */
	get elapsed(): number {
		return this.downloads ? offlinePlayer.position : app.elapsed;
	}

	/** Its length in seconds (0: unknown, or a stream). */
	get duration(): number {
		if (this.downloads) return offlinePlayer.duration || offlinePlayer.current?.duration || 0;
		return app.now?.duration ?? this.track?.duration ?? 0;
	}

	/** A library track's id (lyrics, ratings, waveforms…); downloads are library tracks too. */
	get libraryId(): number | null {
		const t = this.track;
		return typeof t?.id === 'number' && t.id > 0 && !isRemote(t.remote) ? t.id : null;
	}

	/** A download's cover (a blob: URL), or ''. */
	get localCover(): string {
		return this.downloads ? offlinePlayer.cover : '';
	}

	/** The downloads queue (in play order) and where it is. */
	get queue(): Track[] {
		return this.downloads ? offlinePlayer.queue.map((t) => trackOf(t)!) : [];
	}

	get index(): number {
		return this.downloads ? offlinePlayer.index : Number(app.now?.playlist_cur_index ?? -1);
	}

	/** What follows in the downloads queue (the LMS player's Up next is UpNext's own). */
	get nextDownload(): Track | undefined {
		return this.downloads ? trackOf(offlinePlayer.queue[offlinePlayer.index + 1]) : undefined;
	}
}

export const now = new NowView();
