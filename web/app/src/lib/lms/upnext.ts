// "Up next" on Now playing: the track after the current one, from the player's status
// (`status - 2` answers with the current track and the one after it).
import type { PlayerStatus, Track } from './types';

export interface UpNext {
	track: Track;
	/** Its index in the queue (for removing it). */
	index: number;
}

/**
 * The next track, or null: none queued after the current one, "repeat this track" (the
 * same track plays again), or an answer that doesn't line up with the current index.
 */
export function upNext(s: PlayerStatus | null | undefined): UpNext | null {
	const loop = s?.playlist_loop;
	if (!s || !Array.isArray(loop) || loop.length < 2) return null;
	if (Number(s['playlist repeat'] ?? 0) === 1) return null;
	const cur = Number(s.playlist_cur_index ?? NaN);
	if (!Number.isInteger(cur) || cur < 0) return null;
	const next = loop[1];
	if (!next || typeof next !== 'object') return null;
	const at = next['playlist index'];
	if (at !== undefined && Number(at) !== cur + 1) return null;
	return { track: next, index: cur + 1 };
}
