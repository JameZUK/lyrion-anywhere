// Queue sections (Wave 2 §3.4): *Now playing*, *Up next* and *Then*. LMS keeps one flat list,
// so "Up next" (what this device added with Play next) is tracked here: per player, best
// effort, as a run of `k` tracks right after the current one.
//
// - Play next from this device inserts after the current track: the run grows by however
//   many tracks the queue grew by.
// - The current track moving on eats into the run; this device's own edits (remove, move,
//   clear, undo) shift it.
// - Anything else that changes the queue (another app, a reshuffle, going back a track) can't
//   be accounted for, so the run is dropped and the queue shows flat.
// - The queue page checks the run against the tracks it loads: once their ids are known, a
//   run that no longer holds them is dropped too.

export interface QueueState {
	/** How many tracks the queue has. */
	count: number;
	/** The current track's index. */
	cur: number;
	/** LMS's shuffle mode, when known. */
	shuffle?: number;
}

/** How long a Play next waits to show up in the player's status (ms). */
export const PENDING_MS = 15_000;
/** Longest run tracked (more is treated as unaccountable). */
export const MAX_RUN = 5_000;

interface Entry {
	/** The run's length: tracks cur+1 … cur+k. */
	k: number;
	/** The queue as last seen (or as this device's own edits left it). */
	known: QueueState | null;
	/** The run's track ids, once the queue page has seen them. */
	ids: (number | null)[] | null;
	/** A Play next sent, waiting for the queue to grow. */
	pending: { before: QueueState; at: number } | null;
	/** The queue grew without a Play next: keep the run only if its ids still check out. */
	check: boolean;
}

export interface Sections {
	cur: number;
	/** Up next: indices `from` … `to` (inclusive); empty when `to < from`. */
	from: number;
	to: number;
}

function valid(s: QueueState | null | undefined): s is QueueState {
	return !!s && Number.isSafeInteger(s.count) && s.count >= 0 && Number.isSafeInteger(s.cur) && s.cur >= 0 && (s.count === 0 || s.cur < s.count);
}

/** A player's status as a QueueState, or null. */
export function stateOf(s: { playlist_tracks?: unknown; playlist_cur_index?: unknown; 'playlist shuffle'?: unknown } | null | undefined): QueueState | null {
	if (!s) return null;
	const count = Number(s.playlist_tracks ?? 0);
	const cur = Number(s.playlist_cur_index ?? 0);
	const shuffle = s['playlist shuffle'] === undefined ? undefined : Number(s['playlist shuffle']);
	const st: QueueState = { count, cur, ...(shuffle !== undefined && Number.isInteger(shuffle) ? { shuffle } : {}) };
	return valid(st) ? st : null;
}

export class UpNextTracker {
	#by = new Map<string, Entry>();
	/** Bumped on every change (the reactive wrapper watches it). */
	changed: () => void = () => {};

	#entry(player: string): Entry {
		let e = this.#by.get(player);
		if (!e) {
			e = { k: 0, known: null, ids: null, pending: null, check: false };
			this.#by.set(player, e);
		}
		return e;
	}

	#drop(e: Entry) {
		const had = e.k > 0;
		e.k = 0;
		e.ids = null;
		e.check = false;
		if (had) this.changed();
	}

	/** The run's length for `player` (0: the queue shows flat). */
	run(player: string): number {
		return this.#by.get(player)?.k ?? 0;
	}

	/** The sections to show, or null for a flat queue. */
	sections(player: string, cur: number, total: number): Sections | null {
		const e = this.#by.get(player);
		if (!e || e.k <= 0 || !e.known || e.known.cur !== cur || !Number.isInteger(cur) || cur < 0) return null;
		const to = Math.min(cur + e.k, total - 1);
		return to > cur ? { cur, from: cur + 1, to } : null;
	}

	/** This device is about to send Play next; `before` is the queue as it stands. */
	expectInsert(player: string, before: QueueState | null, now = Date.now()) {
		if (!player || !valid(before)) return;
		const e = this.#entry(player);
		e.pending = { before, at: now };
		if (!e.known) e.known = before;
	}

	/** The player's status, as polled. */
	observe(player: string, s: QueueState | null, now = Date.now()) {
		if (!player || !valid(s)) return;
		const e = this.#entry(player);
		const prev = e.known;
		e.known = { ...s };
		const p = e.pending;
		if (p && now - p.at > PENDING_MS) e.pending = null;
		if (!prev) return;
		if (prev.shuffle !== undefined && s.shuffle !== undefined && prev.shuffle !== s.shuffle) {
			e.pending = null;
			return this.#drop(e);
		}
		// A Play next has landed: the tracks went in right after the current one.
		if (e.pending && s.count > e.pending.before.count && s.cur === e.pending.before.cur && e.pending.before.count > 0) {
			const n = s.count - e.pending.before.count;
			// The run must still sit right after the current track for the new tracks to join it.
			if (prev.cur !== s.cur) this.#drop(e);
			e.pending = null;
			if (e.k + n > MAX_RUN) return this.#drop(e);
			e.k += n;
			e.ids = null;
			e.check = false;
			this.changed();
			return;
		}
		if (s.count === prev.count) {
			if (s.cur > prev.cur) {
				// Played on: the run shrinks by what has been reached.
				const d = s.cur - prev.cur;
				if (e.k) {
					e.k = Math.max(0, e.k - d);
					e.ids = e.ids ? e.ids.slice(d) : null;
					if (!e.k) e.ids = null;
					this.changed();
				}
			} else if (s.cur < prev.cur) {
				// Went back: tracks now sit between the current one and the run.
				this.#drop(e);
			}
			return;
		}
		if (e.pending && s.count >= e.pending.before.count && s.cur === prev.cur) return; // still landing
		if (s.count > prev.count && s.cur === prev.cur && e.k && e.ids) {
			// Grown at the end, perhaps (a station topping up): check the run's ids next time.
			e.check = true;
			return;
		}
		this.#drop(e);
	}

	/** The queue page loaded the queue from index 0: check the run against its ids. */
	verify(player: string, cur: number, ids: readonly (number | null | undefined)[]) {
		const e = this.#by.get(player);
		if (!e || !e.k || !e.known || e.known.cur !== cur) return;
		const block = ids.slice(cur + 1, cur + 1 + e.k).map((v) => (typeof v === 'number' ? v : null));
		if (!e.ids) {
			if (e.check) return this.#drop(e);
			e.ids = block;
			return;
		}
		const n = Math.min(block.length, e.ids.length);
		for (let i = 0; i < n; i++) if (block[i] !== e.ids[i]) return this.#drop(e);
		e.check = false;
	}

	/** This device removed the track at `index`. */
	removed(player: string, index: number) {
		const e = this.#by.get(player);
		if (!e?.known || !Number.isInteger(index) || index < 0 || index >= e.known.count) return;
		const { cur } = e.known;
		e.known = { ...e.known, count: e.known.count - 1, cur: index < cur ? cur - 1 : cur };
		if (index > cur && index <= cur + e.k) {
			e.k--;
			e.ids?.splice(index - cur - 1, 1);
			if (!e.k) e.ids = null;
			this.changed();
		} else if (index === cur && e.k) {
			// The current track went: LMS plays on from the next one, which leads the run.
			this.#drop(e);
		}
	}

	/** This device moved the track at `from` so that it ends up at `to` (LMS's `playlist move`). */
	moved(player: string, from: number, to: number) {
		const e = this.#by.get(player);
		if (!e?.known) return;
		const { cur, count } = e.known;
		if (![from, to].every((i) => Number.isInteger(i) && i >= 0 && i < count)) return;
		// Follow the current track and the run through the move.
		const span = Math.max(from, to, cur + e.k) + 1;
		const tags: ('c' | 'b' | '')[] = Array.from({ length: span }, (_, i) => (i === cur ? 'c' : i > cur && i <= cur + e.k ? 'b' : ''));
		const [moving] = tags.splice(from, 1);
		tags.splice(to, 0, moving ?? '');
		const newCur = tags.indexOf('c');
		const before = e.k - (moving === 'b' ? 1 : 0);
		// Moved to just after the current track, or into the run: it joins the run.
		if (moving === '' && to >= newCur + 1 && to <= newCur + 1 + before) tags[to] = 'b';
		let k = 0;
		while (tags[newCur + 1 + k] === 'b') k++;
		e.known = { ...e.known, cur: newCur };
		if (k !== e.k || from !== to) {
			e.k = k;
			e.ids = null;
			e.check = false;
			this.changed();
		}
	}

	/** This device added `n` tracks at the end (e.g. Undo, before moving one into place). */
	appended(player: string, n = 1) {
		const e = this.#by.get(player);
		if (!e?.known || !(n > 0)) return;
		e.known = { ...e.known, count: e.known.count + n };
	}

	/** This device took out every track from `from` on (Clear upcoming, or Clear all at 0). */
	truncated(player: string, from: number) {
		const e = this.#by.get(player);
		if (!e) return;
		if (e.known) e.known = { ...e.known, count: Math.max(0, Math.min(e.known.count, from)), cur: from === 0 ? 0 : e.known.cur };
		this.#drop(e);
	}

	/** Forget one player, or all of them (another user signed in). */
	reset(player?: string) {
		if (player === undefined) this.#by.clear();
		else this.#by.delete(player);
		this.changed();
	}
}

/**
 * Gapless markers (Wave 2 §3.5): `out[i]` is true when row `i` runs straight into row `i + 1`:
 * the same album and the next track number. Presentation only.
 */
export function gaplessLinks(tracks: readonly { album_id?: unknown; tracknum?: unknown }[]): boolean[] {
	const num = (v: unknown) => {
		const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,4}$/.test(v.trim()) ? Number(v) : NaN;
		return Number.isInteger(n) && n > 0 ? n : null;
	};
	const album = (v: unknown) => {
		const n = typeof v === 'number' ? v : typeof v === 'string' && /^\d{1,15}$/.test(v) ? Number(v) : NaN;
		return Number.isSafeInteger(n) && n > 0 ? n : null;
	};
	return tracks.map((t, i) => {
		const u = tracks[i + 1];
		if (!t || !u) return false;
		const a = album(t.album_id);
		const x = num(t.tracknum);
		const y = num(u.tracknum);
		return a !== null && a === album(u.album_id) && x !== null && y === x + 1;
	});
}
