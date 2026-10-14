// The offline player (SPEC §8.4): plays downloaded tracks on this device with a plain
// <audio> element and blob: URLs, with its own queue (album order), lock-screen controls
// (Media Session) and background playback. It is separate from LMS players.
//
// Plays are reported for sync-back (SPEC D6; see ./plays.svelte): when a track finishes, is
// skipped, or playback stops, `onPlay` gets the track, when it started and the seconds
// actually heard (seeking doesn't count as listening).
import type { OfflineStore } from './store';
import { FORMAT_LABELS, type StoredTrack } from './types';

export interface AudioLike extends EventTarget {
	src: string;
	currentTime: number;
	readonly duration: number;
	readonly paused: boolean;
	preload: string;
	play(): Promise<void>;
	pause(): void;
	load(): void;
	canPlayType(type: string): string;
	removeAttribute(name: string): void;
}

export interface PlayerDeps {
	store: OfflineStore;
	createAudio?: () => AudioLike;
	mediaSession?: MediaSession | null;
	urls?: { create(b: Blob): string; revoke(u: string): void };
	/** Builds lock-screen metadata (MediaMetadata where the browser has it). */
	metadata?: (init: MediaMetadataInit) => MediaMetadata | null;
	/** A play ended (finished, skipped or stopped): `seconds` heard, from `startedMs`. */
	onPlay?: (user: string, track: StoredTrack, startedMs: number, seconds: number) => void;
	now?: () => number;
}

/** Larger jumps between position updates are seeks, not listening. */
const MAX_STEP = 2;

/** Other names browsers know a file type by: some say "" to audio/x-flac but play audio/flac. */
const ALIASES: Record<string, string[]> = {
	flac: ['audio/flac'],
	flc: ['audio/flac'],
	mp3: ['audio/mpeg'],
	m4a: ['audio/mp4', 'audio/aac'],
	aac: ['audio/aac', 'audio/mp4'],
	alac: ['audio/mp4'],
	ogg: ['audio/ogg'],
	oga: ['audio/ogg'],
	opus: ['audio/ogg; codecs=opus', 'audio/webm; codecs=opus'],
	wav: ['audio/wav'],
	aif: ['audio/aiff'],
	aiff: ['audio/aiff'],
	wma: [],
};

/** Can this browser play a downloaded track? */
export function playable(audio: Pick<AudioLike, 'canPlayType'>, t: Pick<StoredTrack, 'contentType' | 'ext' | 'codec'>): boolean {
	const types = [t.contentType, ...(ALIASES[t.ext] ?? []), ...(ALIASES[t.codec] ?? [])].filter(Boolean);
	return types.some((type) => {
		try {
			return audio.canPlayType(type) !== '';
		} catch {
			return false;
		}
	});
}

/** What to say when a track can't be played here. */
export function unplayableMessage(t: Pick<StoredTrack, 'format' | 'codec' | 'ext'>): string {
	const what = t.format === 'original' ? (t.codec || t.ext || 'this').toUpperCase() : FORMAT_LABELS[t.format];
	return `This browser can’t play ${what} files. Remove the download and download it again as MP3 or Opus to play it here.`;
}

function defaultMetadata(init: MediaMetadataInit): MediaMetadata | null {
	return typeof MediaMetadata === 'undefined' ? null : new MediaMetadata(init);
}

export class OfflinePlayer {
	queue = $state<StoredTrack[]>([]);
	index = $state(-1);
	playing = $state(false);
	position = $state(0);
	duration = $state(0);
	error = $state('');
	/** The current album's cover as a blob: URL ('' if none). */
	cover = $state('');

	#audio: AudioLike | null = null;
	#user = '';
	#url = '';
	#load = 0;
	#ms: MediaSession | null;
	#urls: { create(b: Blob): string; revoke(u: string): void };
	#coverAlbum: number | null = null;
	/** The play being listened to: its track, start time and seconds heard so far. */
	#play: { user: string; track: StoredTrack; startedMs: number; heard: number; last: number | null } | null = null;

	constructor(private deps: PlayerDeps) {
		this.#ms = deps.mediaSession !== undefined ? deps.mediaSession : typeof navigator !== 'undefined' && 'mediaSession' in navigator ? navigator.mediaSession : null;
		this.#urls = deps.urls ?? { create: (b) => URL.createObjectURL(b), revoke: (u) => URL.revokeObjectURL(u) };
	}

	/** Playing (or paused on) downloads: the lock screen belongs to this player meanwhile. */
	get active(): boolean {
		return this.queue.length > 0;
	}

	/** Whose downloads are queued ('' before the first play). */
	get user(): string {
		return this.#user;
	}

	get current(): StoredTrack | undefined {
		return this.queue[this.index];
	}

	#el(): AudioLike {
		if (this.#audio) return this.#audio;
		let a: AudioLike;
		if (this.deps.createAudio) a = this.deps.createAudio();
		else {
			// In the document (hidden), so mobile browsers treat it as the page's media: that is
			// what keeps it playing in the background with lock-screen controls.
			const el = document.createElement('audio');
			el.hidden = true;
			el.setAttribute('playsinline', '');
			el.setAttribute('aria-hidden', 'true');
			document.body.append(el);
			a = el as unknown as AudioLike;
		}
		a.preload = 'auto';
		a.addEventListener('timeupdate', () => {
			const cur = a.currentTime || 0;
			this.position = cur;
			const p = this.#play;
			if (p) {
				// A track starts at 0, so the first update counts from there.
				const step = cur - (p.last ?? 0);
				if (this.playing && step > 0 && step <= MAX_STEP) p.heard += step;
				p.last = cur;
			}
		});
		a.addEventListener('durationchange', () => {
			if (Number.isFinite(a.duration) && a.duration > 0) this.duration = a.duration;
			this.#positionState();
		});
		a.addEventListener('play', () => {
			if (this.#play && !this.#play.startedMs) this.#play.startedMs = this.#now();
			this.#state(true);
		});
		a.addEventListener('pause', () => this.#state(false));
		a.addEventListener('ended', () => void this.#finished());
		a.addEventListener('error', () => {
			if (!this.#url) return;
			this.error = 'This track couldn’t be played. The download may be damaged: remove it and download it again.';
			this.#state(false);
		});
		this.#audio = a;
		return a;
	}

	#now(): number {
		return this.deps.now ? this.deps.now() : Date.now();
	}

	/** Start counting a play of the current track. */
	#begin(t: StoredTrack) {
		this.#play = { user: this.#user, track: t, startedMs: 0, heard: 0, last: null };
	}

	/** Report the play in progress, if anything was heard. */
	flushPlay() {
		const p = this.#play;
		this.#play = null;
		if (!p || p.heard <= 0 || !p.user) return;
		try {
			this.deps.onPlay?.(p.user, p.track, p.startedMs || this.#now(), p.heard);
		} catch {
			/* reporting must never stop playback */
		}
	}

	#state(playing: boolean) {
		this.playing = playing;
		if (this.#ms) this.#ms.playbackState = this.active ? (playing ? 'playing' : 'paused') : 'none';
		this.#positionState();
	}

	#positionState() {
		const d = this.duration;
		if (!this.#ms?.setPositionState || !(d > 0)) return;
		try {
			this.#ms.setPositionState({ duration: d, position: Math.min(this.position, d), playbackRate: 1 });
		} catch {
			/* not supported */
		}
	}

	/** Play a list of downloaded tracks (an album, in order) from `start`. */
	async playList(user: string, tracks: StoredTrack[], start = 0): Promise<void> {
		if (!tracks.length) return;
		this.#user = user;
		this.queue = tracks.map((t) => ({ ...t }));
		this.#handlers(true);
		await this.#loadIndex(Math.max(0, Math.min(start, tracks.length - 1)), true);
	}

	async #loadIndex(i: number, autoplay: boolean): Promise<void> {
		const t = this.queue[i];
		if (!t) return;
		const load = ++this.#load;
		const a = this.#el();
		this.flushPlay();
		this.#begin(t);
		this.index = i;
		this.position = 0;
		this.duration = t.duration || 0;
		this.error = '';
		a.pause();
		this.#release();
		void this.#metadata(t, load);
		if (!playable(a, t)) {
			this.error = unplayableMessage(t);
			this.#state(false);
			return;
		}
		let blob: Blob | undefined;
		try {
			blob = await this.deps.store.getAudio(this.#user, t.key);
		} catch {
			blob = undefined;
		}
		if (load !== this.#load) return; // another track was chosen meanwhile
		if (!blob) {
			this.error = 'This download is missing or incomplete. Remove it and download it again.';
			this.#state(false);
			return;
		}
		this.#url = this.#urls.create(blob);
		a.src = this.#url;
		if (!autoplay) return;
		try {
			await a.play();
		} catch (e) {
			if (load !== this.#load) return;
			if ((e as Error)?.name === 'NotAllowedError') this.error = 'Tap play to start.';
			else if ((e as Error)?.name !== 'AbortError') this.error = 'This track couldn’t be played.';
			this.#state(false);
		}
	}

	async #metadata(t: StoredTrack, load: number) {
		// The album cover, from storage: works with no connection.
		if (t.albumId !== this.#coverAlbum || !this.cover) {
			if (this.cover) this.#urls.revoke(this.cover);
			this.cover = '';
			this.#coverAlbum = t.albumId;
			if (t.albumId !== null) {
				const blob = await this.deps.store.getCover(this.#user, t.albumId).catch(() => undefined);
				if (load !== this.#load) return;
				if (blob) this.cover = this.#urls.create(blob);
			}
		}
		if (!this.#ms || load !== this.#load) return;
		const make = this.deps.metadata ?? defaultMetadata;
		this.#ms.metadata = make({ title: t.title, artist: t.artist, album: t.album, artwork: this.cover ? [{ src: this.cover }] : [] });
	}

	#handlers(on: boolean) {
		const ms = this.#ms;
		if (!ms) return;
		const actions: [MediaSessionAction, MediaSessionActionHandler | null][] = [
			['play', on ? () => void this.resume() : null],
			['pause', on ? () => this.pause() : null],
			['nexttrack', on ? () => void this.next() : null],
			['previoustrack', on ? () => void this.previous() : null],
			['seekto', on ? (d) => typeof d.seekTime === 'number' && this.seek(d.seekTime) : null],
			['seekbackward', on ? (d) => this.seek(this.position - (d.seekOffset ?? 10)) : null],
			['seekforward', on ? (d) => this.seek(this.position + (d.seekOffset ?? 10)) : null],
			['stop', on ? () => this.stop() : null],
		];
		for (const [a, fn] of actions) {
			try {
				ms.setActionHandler(a, fn);
			} catch {
				/* unsupported action */
			}
		}
	}

	async #finished() {
		if (this.index < this.queue.length - 1) await this.#loadIndex(this.index + 1, true);
		else {
			// The last track finished: that play is over; playing it again is a new one.
			this.flushPlay();
			if (this.current) this.#begin(this.current);
			this.#state(false);
			this.position = 0;
			if (this.#audio) this.#audio.currentTime = 0;
		}
	}

	async resume(): Promise<void> {
		const a = this.#audio;
		if (!a || !this.active) return;
		if (!this.#url) return this.#loadIndex(this.index, true);
		try {
			await a.play();
			this.error = '';
		} catch {
			this.#state(false);
		}
	}

	pause() {
		this.#audio?.pause();
	}

	toggle() {
		if (this.playing) this.pause();
		else void this.resume();
	}

	async next(): Promise<void> {
		if (this.index < this.queue.length - 1) await this.#loadIndex(this.index + 1, true);
	}

	/** Back to the start of the track, or (near its start) the track before. */
	async previous(): Promise<void> {
		if (this.position > 3 || this.index === 0) this.seek(0);
		else await this.#loadIndex(this.index - 1, true);
	}

	async jump(i: number): Promise<void> {
		if (i >= 0 && i < this.queue.length) await this.#loadIndex(i, true);
	}

	seek(seconds: number) {
		const a = this.#audio;
		if (!a || !Number.isFinite(seconds)) return;
		const max = this.duration > 0 ? this.duration : Number.POSITIVE_INFINITY;
		const t = Math.max(0, Math.min(seconds, max));
		a.currentTime = t;
		this.position = t;
		this.#positionState();
	}

	#release() {
		const a = this.#audio;
		if (this.#url) {
			if (a) {
				a.removeAttribute('src');
				a.load();
			}
			this.#urls.revoke(this.#url);
			this.#url = '';
		}
	}

	/** Stop, forget the queue, and give back every blob: URL and the lock-screen controls. */
	stop() {
		this.flushPlay();
		this.#load++;
		this.#audio?.pause();
		this.#release();
		if (this.cover) this.#urls.revoke(this.cover);
		this.cover = '';
		this.#coverAlbum = null;
		this.queue = [];
		this.index = -1;
		this.position = 0;
		this.duration = 0;
		this.error = '';
		this.playing = false;
		this.#handlers(false);
		if (this.#ms) {
			this.#ms.metadata = null;
			this.#ms.playbackState = 'none';
		}
	}

	/** Drop tracks that were removed from the device; stop if the current one went. */
	forget(keys: string[]) {
		if (!this.active) return;
		if (this.current && keys.includes(this.current.key)) return this.stop();
		const cur = this.current;
		this.queue = this.queue.filter((t) => !keys.includes(t.key));
		this.index = cur ? this.queue.findIndex((t) => t.key === cur.key) : -1;
	}
}
