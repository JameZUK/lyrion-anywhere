// This browser as the player while the server is out of reach (SPEC.md, "Controls while the
// server is out of reach"). The device's own stream keeps playing from memory; Play/Pause
// holds or lets go of its output (the position is kept); Next, Previous and the queue jump
// to entries this device has: a download (played here from storage) or the stream it holds.
// When the server is back, what was done here is told to LMS, if its queue still has the
// same track at that place; otherwise LMS is followed. Nothing done here: nothing is told.
//
// The decisions are in ./local-plan (pure); this is the glue around them.
import type { Command } from '#lib/api/rpc';
import { describePageCommand } from './diagnostics';
import * as cmd from '#lib/lms/commands';
import type { PlayerStatus, Track } from '#lib/lms/types';
import { playable, type AudioLike } from '#lib/offline/player.svelte';
import type { StoredTrack } from '#lib/offline/types';
import { NEXT_NEEDS_SERVER, afterEnd, nextIndex, previousTarget, queueMark, reconcile, type LmsNow, type LocalKind, type QueueState } from './local-plan';

/** The element downloads play through (an <audio>; a fake in tests). */
export type LocalAudio = AudioLike & { volume: number; readonly ended: boolean };

/** How much of the queue around the current entry is kept, for playing on with no server. */
export const SNAPSHOT_BEFORE = 50;
export const SNAPSHOT_SIZE = 250;

/** This device's player, as far as playing on without the server goes. */
export interface LocalDevice {
	mac(): string;
	/** The core's output state ('Playing', 'Paused { … }', 'Stopped', …). */
	output(): string;
	/** A pause made here holds the output. */
	held(): boolean;
	/** The stream being fetched has all arrived. */
	received(): boolean;
	/** The core's sequence number of the track it last handed to the output. */
	track(): number | null;
	/** The last connection was a fresh session (LMS saw a new player). */
	fresh(): boolean;
	local(m: { hold: true } | { release: 'now' | 'flush' | 'paused' }): void;
	/** A line for this device's connection log. */
	note?(msg: string): void;
}

export interface LocalDeps {
	device: LocalDevice;
	/** LMS's last status for this device (as the app shows it). */
	status(): PlayerStatus | null;
	/** Show what plays here (merged into the app's status). */
	apply(patch: PlayerStatus): void;
	/** The play head the app shows now (seconds). */
	elapsed(): number;
	/** The downloaded copy of an LMS track, if any. */
	downloaded(id: unknown): StoredTrack | undefined;
	/** A download's audio, from storage. */
	audio(t: StoredTrack): Promise<Blob | undefined>;
	rpc<T = Record<string, unknown>>(player: string, c: Command): Promise<T>;
	/** Once the server has been told (or followed): refresh the app's status. */
	refreshed(): void;
	createAudio?: () => LocalAudio;
	urls?: { create(b: Blob): string; revoke(u: string): void };
	/** The volume to play downloads at (0–1), to match this device's output. */
	volume?: () => number;
}

type Source = 'stream' | 'download' | null;

export class LocalPlayback {
	/** Playing here without the server (this device is the player, the server out of reach). */
	engaged = $state(false);
	/** The queue entry playing here. */
	index = $state(-1);
	/** What plays: the device's stream (from memory), a download, or nothing. */
	source = $state<Source>(null);
	paused = $state(false);
	/** Why playback stopped (the next track isn't on this device), or ''. */
	waiting = $state('');
	/** The queue around the current entry, as last read (index → track). */
	rows = $state(new Map<number, Track>());
	/** Bumped whenever what is available here may have changed (for marks and gates). */
	version = $state(0);

	/** The entry whose stream the device holds (-1: none), and how far into it it was left. */
	#streamIndex = -1;
	#streamTime = 0;
	/** Something was done here: the server must be told when it is back. */
	#acted = false;
	#lastOutput = '';
	#lastTrack: number | null = null;
	#audio: LocalAudio | null = null;
	#url = '';
	#load = 0;
	#reconciling = false;
	/** Done here, but the server hasn't been told yet (it is still out of reach). */
	#pending = false;
	/** Where a download stopped when playing here ended before the server was back. */
	#stoppedAt: number | null = null;

	constructor(private deps: LocalDeps) {}

	#queue(): QueueState {
		const s = this.deps.status();
		return { index: this.index, count: Number(s?.playlist_tracks ?? 0) || 0, repeat: Number(s?.['playlist repeat'] ?? 0) || 0 };
	}

	/** The device still has audio of its stream to play (it hasn't run out or been stopped). */
	get streamAlive(): boolean {
		return this.#streamIndex >= 0 && this.deps.device.output() !== 'Stopped';
	}

	/** Can entry `i` be played here from its start (a download this browser can play)? */
	fromStart = (i: number): boolean => {
		void this.version;
		const t = this.rows.get(i);
		const d = t ? this.deps.downloaded(t.id) : undefined;
		return !!d && this.#canPlay(d);
	};

	/** Can entry `i` be played here: a download, or the stream the device holds? */
	available = (i: number): boolean => this.fromStart(i) || (i === this.#streamIndex && this.streamAlive);

	#canPlay(t: StoredTrack): boolean {
		try {
			return playable(this.#el(), t);
		} catch {
			return false;
		}
	}

	/**
	 * The mark for queue entry `index` (track `id`): downloads for any player; the stream this
	 * device holds whole, only while it is the player (`current` is LMS's current entry).
	 */
	mark(index: number, id: unknown, thisDevice: boolean, current: number): LocalKind | null {
		void this.version;
		const held = this.engaged ? (this.streamAlive && this.deps.device.received() ? this.#streamIndex : -1) : this.deps.device.received() && this.deps.device.output() !== 'Stopped' ? current : -1;
		return queueMark({ id, index, downloaded: (x) => this.deps.downloaded(x) !== undefined, thisDevice, heldIndex: held });
	}

	/** Keep the queue around `current` while the server answers (read before going out of reach). */
	async snapshot(player: string, current: number): Promise<void> {
		const from = Math.max(0, current - SNAPSHOT_BEFORE);
		const r = await this.deps.rpc<PlayerStatus>(player, cmd.queue(from, SNAPSHOT_SIZE));
		if (this.engaged) return; // out of reach meanwhile: keep what was there
		const rows = new Map<number, Track>();
		(r?.playlist_loop ?? []).forEach((t, k) => rows.set(from + k, t));
		this.rows = rows;
	}

	/** Downloads changed: what is available may have too. */
	touch() {
		this.version++;
	}

	/**
	 * Start or stop playing here on its own: `on` when this device is the player and the
	 * server is out of reach. Going off with something done here tells the server (once
	 * `online`); going off because another player was chosen just drops it.
	 */
	sync(on: boolean, online: boolean) {
		if (on) {
			if (!this.engaged) this.#engage();
			return;
		}
		if (this.engaged) {
			this.engaged = false;
			if (!this.#acted) return this.#reset('now');
			// Another player was chosen while out of reach: stop here (a pause, as far as the
			// server will hear), and tell it once it is back.
			if (!online && this.source === 'download') {
				this.#stoppedAt = this.#audio?.currentTime || 0;
				this.#stopAudio();
				this.paused = true;
			}
			this.#pending = true;
		}
		if (this.#pending && online) {
			this.#pending = false;
			void this.#reconcile();
		}
	}

	#engage() {
		const s = this.deps.status();
		const index = Number(s?.playlist_cur_index ?? -1);
		this.index = Number.isInteger(index) ? index : -1;
		// The current track, as the status has it, in case the queue was never read.
		const cur = s?.playlist_loop?.[0];
		if (cur && this.index >= 0 && !this.rows.has(this.index)) this.rows = new Map(this.rows).set(this.index, cur);
		const out = this.deps.device.output();
		this.#streamIndex = out && out !== 'Stopped' ? this.index : -1;
		this.#streamTime = this.deps.elapsed();
		this.source = this.#streamIndex >= 0 ? 'stream' : null;
		this.paused = s?.mode !== 'play';
		this.waiting = '';
		this.#acted = this.#pending; // still to be told from before
		this.#pending = false;
		this.#lastOutput = out;
		this.#lastTrack = this.deps.device.track();
		this.engaged = true;
	}

	/** Forget what was done here, and give the output back to LMS. */
	#reset(release: 'now' | 'flush' | 'paused') {
		this.#stopAudio();
		if (this.deps.device.held()) this.deps.device.local({ release });
		this.source = null;
		this.index = -1;
		this.#streamIndex = -1;
		this.waiting = '';
		this.#acted = false;
		this.#pending = false;
		this.#stoppedAt = null;
	}

	// --- controls ----------------------------------------------------------------------

	toggle() {
		if (!this.engaged) return;
		this.#acted = true;
		if (this.source === 'download') {
			const a = this.#audio;
			if (!a) return;
			if (a.paused) void a.play().catch(() => {});
			else a.pause();
			return;
		}
		if (this.source === 'stream') {
			if (this.deps.device.held()) {
				this.deps.device.local({ release: 'now' });
				this.#show({ mode: 'play', time: this.deps.elapsed() });
			} else if (this.deps.device.output() === 'Playing') {
				this.deps.device.local({ hold: true });
				this.#show({ mode: 'pause', time: this.deps.elapsed() });
			}
			return;
		}
		// Nothing playing yet: start the current track from the download, where LMS left it.
		if (this.fromStart(this.index)) void this.#playDownload(this.index, this.deps.elapsed());
	}

	next() {
		if (!this.engaged) return;
		const i = nextIndex(this.#queue(), this.available);
		if (i !== null) this.#goTo(i, 0);
	}

	previous() {
		if (!this.engaged) return;
		const p = previousTarget(this.#queue(), this.deps.elapsed(), this.available, this.fromStart);
		if (!p) return;
		if ('restart' in p) {
			if (this.source === 'download') this.seek(0);
			else this.#goTo(this.index, 0, true);
		} else this.#goTo(p.index, 0);
	}

	jump(i: number) {
		if (!this.engaged || !this.available(i)) return;
		if (i === this.index && this.source !== null) return;
		this.#goTo(i, 0);
	}

	/** Seek within a download (a stream can't be sought without LMS). */
	seek(seconds: number) {
		const a = this.#audio;
		if (!this.engaged || this.source !== 'download' || !a || !Number.isFinite(seconds)) return;
		this.#acted = true;
		const t = Math.max(0, seconds);
		a.currentTime = t;
		this.#show({ time: t });
	}

	/** Play entry `i`: the device's stream where it was left, or a download from `at`. */
	#goTo(i: number, at: number, fromStartOnly = false) {
		this.#acted = true;
		this.waiting = '';
		if (!fromStartOnly && i === this.#streamIndex && this.streamAlive) {
			this.#stopAudio();
			this.source = 'stream';
			this.index = i;
			if (this.deps.device.held()) this.deps.device.local({ release: 'now' });
			this.#show({ mode: 'play', time: this.#streamTime });
			return;
		}
		if (this.fromStart(i)) void this.#playDownload(i, at);
	}

	async #playDownload(i: number, at: number) {
		const row = this.rows.get(i);
		const stored = row ? this.deps.downloaded(row.id) : undefined;
		if (!row || !stored) return;
		const load = ++this.#load;
		// Leave the stream held where it was, to come back to.
		if (this.source === 'stream') {
			this.#streamTime = this.deps.elapsed();
			if (!this.deps.device.held()) this.deps.device.local({ hold: true });
		}
		this.#acted = true;
		this.source = 'download';
		this.index = i;
		this.paused = false;
		this.#show({ mode: 'play', time: at, duration: stored.duration || Number(row.duration ?? 0) || 0 });
		let blob: Blob | undefined;
		try {
			blob = await this.deps.audio(stored);
		} catch {
			blob = undefined;
		}
		if (load !== this.#load || !this.engaged) return;
		if (!blob) {
			this.#ended();
			return;
		}
		const a = this.#el();
		this.#release();
		this.#url = this.#urls().create(blob);
		a.src = this.#url;
		a.volume = Math.max(0, Math.min(1, this.deps.volume?.() ?? 1));
		if (at > 0) {
			try {
				a.currentTime = at;
			} catch {
				/* set once it has loaded (below) */
			}
		}
		try {
			await a.play();
			if (at > 0 && Math.abs(a.currentTime - at) > 1) a.currentTime = at;
		} catch {
			if (load === this.#load) this.#show({ mode: 'pause' });
		}
	}

	/** Shown in the app as the status (title, queue position, play head). */
	#show(p: PlayerStatus) {
		const i = this.index;
		const patch: PlayerStatus = { ...p, playlist_cur_index: i };
		const cur = this.rows.get(i);
		if (cur) patch.playlist_loop = [cur, ...(this.rows.has(i + 1) ? [this.rows.get(i + 1)!] : [])];
		if (p.mode) this.paused = p.mode !== 'play';
		this.deps.apply(patch);
	}

	/** The track playing here ended: on to the next one here, or stop and say why. */
	#ended() {
		const next = afterEnd(this.#queue(), this.available, this.fromStart);
		if (next !== null) {
			this.#goTo(next, 0, next === this.index);
			return;
		}
		this.#stopAudio();
		this.source = this.source === 'stream' && this.streamAlive ? 'stream' : null;
		this.waiting = NEXT_NEEDS_SERVER;
		this.#show({ mode: 'stop', time: 0 });
	}

	/** Every status from the device: follow its stream to its end, or on into the next track. */
	onDevice() {
		if (!this.engaged) return;
		const out = this.deps.device.output();
		const track = this.deps.device.track();
		const was = this.#lastOutput;
		const lastTrack = this.#lastTrack;
		this.#lastOutput = out;
		this.#lastTrack = track;
		if (this.source !== 'stream') return;
		if (was && was !== 'Stopped' && out === 'Stopped') {
			// The stream played out with nothing more from LMS.
			this.#streamIndex = -1;
			this.#ended();
			return;
		}
		if (track !== null && lastTrack !== null && track > lastTrack && this.#rowsHave(this.index + 1)) {
			// The device had the next track's stream too, and has gone on into it.
			this.index += 1;
			this.#streamIndex = this.index;
			this.#show({ time: 0 });
		}
	}

	#rowsHave(i: number): boolean {
		return this.rows.has(i);
	}

	/** Back in reach after acting here: tell LMS (or follow it), then let the output go. */
	async #reconcile() {
		if (this.#reconciling) return;
		this.#reconciling = true;
		const mac = this.deps.device.mac();
		const a = this.#audio;
		const time = this.#stoppedAt ?? (this.source === 'download' && a && this.#url ? a.currentTime || 0 : this.deps.elapsed());
		// Stopped at the end of a track for want of the next one: the server plays it now.
		const index = this.waiting ? this.index + 1 : this.index;
		const outcome = {
			index,
			track: this.rows.get(index)?.id,
			time: this.waiting ? 0 : time,
			paused: this.waiting ? false : this.paused,
			sameStream: !this.waiting && this.source === 'stream' && !this.deps.device.fresh(),
		};
		let release: 'now' | 'flush' | 'paused' = 'now';
		try {
			this.#stopAudio();
			if (!mac || outcome.index < 0) return;
			const now = await this.deps.rpc<LmsNow>(mac, cmd.status());
			const q = await this.deps.rpc<{ playlist_loop?: { id?: unknown }[] }>(mac, cmd.queue(outcome.index, 1));
			const plan = reconcile(outcome, now, q?.playlist_loop?.[0]?.id);
			if (!plan.follow) {
				for (const c of plan.commands) {
					this.deps.device.note?.(describePageCommand(c, 'handing back to LMS after playing on without it'));
					await this.deps.rpc(mac, c);
				}
				release = plan.release;
			}
		} catch {
			release = 'now'; // couldn't tell it: follow LMS
		} finally {
			this.#reconciling = false;
			if (!this.engaged) this.#reset(release);
			this.deps.refreshed();
		}
	}

	// --- the element downloads play through ----------------------------------------------

	#urls() {
		return this.deps.urls ?? { create: (b: Blob) => URL.createObjectURL(b), revoke: (u: string) => URL.revokeObjectURL(u) };
	}

	#el(): LocalAudio {
		if (this.#audio) return this.#audio;
		let a: LocalAudio;
		if (this.deps.createAudio) a = this.deps.createAudio();
		else {
			// In the document, so mobile browsers keep it playing in the background.
			const el = document.createElement('audio');
			el.hidden = true;
			el.setAttribute('playsinline', '');
			el.setAttribute('aria-hidden', 'true');
			document.body.append(el);
			a = el as unknown as LocalAudio;
		}
		a.preload = 'auto';
		a.addEventListener('timeupdate', () => {
			if (this.source === 'download' && this.#url) this.#show({ time: a.currentTime || 0 });
		});
		a.addEventListener('play', () => this.source === 'download' && this.#show({ mode: 'play', time: a.currentTime || 0 }));
		a.addEventListener('pause', () => this.source === 'download' && this.#url && !a.ended && this.#show({ mode: 'pause', time: a.currentTime || 0 }));
		a.addEventListener('ended', () => this.source === 'download' && this.#ended());
		this.#audio = a;
		return a;
	}

	#release() {
		if (!this.#url) return;
		const a = this.#audio;
		if (a) {
			a.removeAttribute('src');
			a.load();
		}
		this.#urls().revoke(this.#url);
		this.#url = '';
	}

	#stopAudio() {
		this.#load++;
		this.#audio?.pause();
		this.#release();
	}
}
