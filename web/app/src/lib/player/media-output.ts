// Media-element output: fMP4/FLAC segments from the player worker, played by an <audio>
// element through Managed Media Source (iOS 17.1+) or classic Media Source.
//
// iOS keeps a playing media element going when the page is in the background, which Web
// Audio does not get. Segments must be appended in order, one at a time; played audio is
// trimmed so memory stays bounded.
import { MEDIA_TYPE } from './mode';

type SourceCtor = new () => MediaSource;

export interface MediaOutputOptions {
	source: 'managed' | 'classic';
	/** MIME type the browser accepted (spelling differs between browsers). */
	mime?: string;
	/** Called with short status notes for the connection log. */
	note?: (msg: string) => void;
	/** Called once if the element can't play the stream (the caller falls back). */
	onFail?: (why: string) => void;
	/**
	 * Open the source but don't call play() at start: the browser is known to refuse it
	 * without a gesture, so the caller asks for a tap first (see {@link MediaOutput.resume}).
	 */
	deferPlay?: boolean;
	makeAudio?: () => HTMLAudioElement;
	ctors?: { managed?: SourceCtor; classic?: SourceCtor };
}

/** Keep this many seconds behind the play head; older audio is removed. */
const KEEP_BEHIND = 10;
/** Less than this left to play (seconds) counts as having played everything appended. */
const END_SLACK = 0.1;
/** A play head stuck at most this far (seconds) before buffered audio is moved onto it. */
const MAX_GAP = 0.5;
/** An aborted play() is tried again at most this often before the user is asked to tap. */
const MAX_RETRIES = 8;

export class MediaOutput {
	readonly audio: HTMLAudioElement;
	#ms: MediaSource | null = null;
	#sb: SourceBuffer | null = null;
	#url = '';
	/** Segments to append and cuts to make ({@link trimFrom}), in order. */
	#queue: (Uint8Array | { removeFrom: number })[] = [];
	#closed = false;
	#note: (m: string) => void;
	#failed = false;
	/** Called when the element has played everything appended and is waiting for more. */
	onDrained: (() => void) | null = null;
	/**
	 * Called when something other than this output paused the element: the system (a phone
	 * call, another app's audio, Siri) or the browser. Nothing else would notice: the player
	 * would carry on as if playing, silently.
	 */
	onExternalPause: (() => void) | null = null;
	/** This output wants the element playing (it called play() and not pause() since). */
	#wantPlay = false;
	/** The browser refused play() for want of a gesture: only a tap's play() can start it. */
	#blocked = false;
	/** A play() was aborted (a load or source change meanwhile): try again when ready. */
	#retrying = false;
	#retries = 0;
	/** A play() is in flight. */
	#playPending = false;
	/** Where the element stopped playing (null: playing, or it never started). */
	#pausedAt: number | null = null;
	/** Called when a play() this output made has really started the element. */
	onPlaying: (() => void) | null = null;
	/**
	 * Called when the browser refused to play without a user gesture (NotAllowedError), or
	 * retries kept failing: the caller asks for a tap. Nothing is heard until then.
	 */
	onBlocked: (() => void) | null = null;

	constructor(private opts: MediaOutputOptions) {
		this.audio = (opts.makeAudio ?? (() => new Audio()))();
		this.#note = opts.note ?? (() => {});
	}

	/**
	 * Start playback. Call from the user's tap: the element's play() must start inside the
	 * gesture, even though data only arrives later.
	 */
	async start(): Promise<void> {
		const g = globalThis as unknown as { ManagedMediaSource?: SourceCtor; MediaSource?: SourceCtor };
		const Ctor = this.opts.source === 'managed' ? (this.opts.ctors?.managed ?? g.ManagedMediaSource) : (this.opts.ctors?.classic ?? g.MediaSource);
		if (!Ctor) throw new Error(`${this.opts.source} Media Source is not available`);
		const a = this.audio;
		// Managed Media Source needs an AirPlay alternative or remote playback disabled.
		(a as HTMLAudioElement & { disableRemotePlayback: boolean }).disableRemotePlayback = true;
		a.setAttribute('playsinline', '');
		a.setAttribute('aria-hidden', 'true');
		a.preload = 'auto';
		a.hidden = true;
		document.body.append(a); // in the document, so the browser treats it as page media
		const ms = new Ctor();
		this.#ms = ms;
		this.#url = URL.createObjectURL(ms);
		a.src = this.#url;
		// Inside the gesture; resolves once there is data to play.
		if (this.opts.deferPlay) {
			this.#wantPlay = true;
			this.#blocked = true;
		} else {
			void this.#play('start');
		}
		await new Promise<void>((resolve, reject) => {
			ms.addEventListener('sourceopen', () => resolve(), { once: true });
			setTimeout(() => reject(new Error('media source did not open')), 4_000);
		});
		const sb = ms.addSourceBuffer(this.opts.mime ?? MEDIA_TYPE);
		sb.mode = 'segments';
		sb.addEventListener('updateend', () => {
			this.#retry('segment');
			this.#pump();
		});
		sb.addEventListener('error', () => {
			this.#note('media buffer error');
			this.#fail('the browser rejected the audio');
		});
		this.#sb = sb;
		ms.addEventListener('startstreaming', () => this.#note('media source: start streaming'));
		ms.addEventListener('endstreaming', () => this.#note('media source: end streaming'));
		a.addEventListener('error', () => {
			this.#note(`media element error ${a.error?.code ?? ''}: ${a.error?.message ?? ''}`);
			this.#fail(`media error ${a.error?.code ?? ''}`);
		});
		ms.addEventListener('sourceclose', () => this.#note('media source closed'));
		a.addEventListener('playing', () => (this.#pausedAt = null));
		a.addEventListener('pause', () => {
			this.#pausedAt ??= a.currentTime;
			if (!this.#wantPlay || this.#closed || this.#failed) return;
			this.#wantPlay = false;
			this.#note('the system paused the media element');
			this.onExternalPause?.();
		});
		// The element is ready again after a load or source change: an aborted play() goes again.
		for (const type of ['loadedmetadata', 'canplay'] as const) a.addEventListener(type, () => this.#retry(type));
		a.addEventListener('waiting', () => {
			if (this.atEnd) this.onDrained?.();
			else this.unstick();
		});
		this.#pump();
	}

	#fail(why: string) {
		if (this.#failed || this.#closed) return;
		this.#failed = true;
		this.opts.onFail?.(why);
	}

	/** Queue a segment (init or media) for appending, in order. */
	append(bytes: Uint8Array) {
		if (this.#closed) return;
		this.#queue.push(bytes);
		this.#pump();
	}

	#pump() {
		const sb = this.#sb;
		if (!sb || sb.updating || this.#closed) return;
		const behind = this.audio.currentTime - KEEP_BEHIND;
		if (behind > 1 && sb.buffered.length && sb.buffered.start(0) < behind - 1) {
			try {
				sb.remove(0, behind); // trim played audio; the queue resumes on updateend
				return;
			} catch {
				/* fall through to appending */
			}
		}
		const next = this.#queue.shift();
		if (!next) return;
		if (!(next instanceof Uint8Array)) {
			this.#cut(sb, next.removeFrom);
			return;
		}
		try {
			sb.appendBuffer(next as BufferSource);
		} catch (e) {
			if ((e as DOMException).name === 'QuotaExceededError') {
				this.#queue.unshift(next); // retry after the next trim
				this.#note('media buffer full; waiting');
			} else {
				this.#note(`append failed: ${(e as Error).message}`);
			}
		}
	}

	/**
	 * LMS dropped the tracks after the one playing (`strm f`): remove the audio from timeline
	 * second `t` on, once the segments queued before this call are in. The worker sends the
	 * kept audio after `t` again, then whatever comes next.
	 */
	trimFrom(t: number) {
		if (this.#closed || !Number.isFinite(t) || t < 0) return;
		this.#queue.push({ removeFrom: t });
		this.#pump();
	}

	#cut(sb: SourceBuffer, t: number) {
		const b = sb.buffered;
		if (!b.length || b.end(b.length - 1) <= t) {
			this.#pump(); // nothing there to remove
			return;
		}
		try {
			sb.remove(t, Infinity); // the queue resumes on updateend
		} catch (e) {
			this.#note(`could not remove the dropped track: ${(e as Error).message}`);
			this.#pump();
			return;
		}
		// Already playing past the cut (it came late): go back to where the new audio goes.
		if (this.audio.currentTime > t) this.seek(t);
	}

	/** Play-head position, in timeline seconds. */
	get position(): number {
		return this.audio.currentTime;
	}

	/** End of the buffered audio, in timeline seconds (0 if nothing is buffered). */
	get bufferedEnd(): number {
		const b = this.#sb?.buffered;
		return b && b.length ? b.end(b.length - 1) : 0;
	}

	/** Seconds buffered ahead of the play head. */
	get ahead(): number {
		const b = this.#sb?.buffered;
		if (!b) return 0;
		const t = this.audio.currentTime;
		for (let i = 0; i < b.length; i++) if (b.start(i) <= t + 0.1 && b.end(i) > t) return b.end(i) - t;
		return 0;
	}

	/**
	 * Playing, and the play head has reached the end of everything appended: nothing is
	 * waiting to be appended and less than {@link END_SLACK} is left in the last buffered
	 * range. Only then has all the audio handed to the element been played. A stall anywhere
	 * else (the play head in a gap, before audio still being appended, or the element paused)
	 * is not the end, and counting it as one would tell LMS that audio still to come had
	 * played, and so that the track had finished.
	 */
	get atEnd(): boolean {
		const sb = this.#sb;
		if (!sb || this.#closed || this.audio.paused || sb.updating || this.#queue.length) return false;
		const b = sb.buffered;
		if (!b.length) return false;
		const t = this.audio.currentTime;
		const last = b.length - 1;
		return b.start(last) <= t + END_SLACK && b.end(last) - t < END_SLACK;
	}

	/**
	 * The play head is stuck just before buffered audio (a sliver of a gap, e.g. after a seek
	 * landed a hair before the new audio): move it onto that audio. Returns whether it moved.
	 */
	unstick(): boolean {
		const sb = this.#sb;
		if (!sb || this.#closed || this.audio.paused) return false;
		const b = sb.buffered;
		const t = this.audio.currentTime;
		for (let i = 0; i < b.length; i++) {
			if (b.start(i) <= t && b.end(i) > t) return false; // in buffered audio: not stuck in a gap
			if (b.start(i) > t && b.start(i) - t <= MAX_GAP) {
				this.seek(b.start(i));
				this.#note(`media: moved the play head over a ${Math.round((b.start(i) - t) * 1000)} ms gap`);
				return true;
			}
		}
		return false;
	}

	pause() {
		this.#wantPlay = false;
		this.#retrying = false;
		if (!this.audio.paused) this.#pausedAt ??= this.audio.currentTime;
		this.audio.pause();
	}

	/**
	 * While paused: the timeline position it stopped playing at, so the caller counts what
	 * was heard up to there and not a play head moved since. Null if it never played.
	 */
	get pausedAt(): number | null {
		return this.#pausedAt;
	}

	/**
	 * Play the element. Resolves with whether it really plays: an aborted play() is tried
	 * again by itself when the element is ready ({@link onPlaying} says when it plays); a
	 * refused one ({@link onBlocked}) waits for a tap's call to this.
	 */
	resume(): Promise<boolean> {
		return this.#play('resume');
	}

	/** The element really plays (not merely asked to). */
	get playing(): boolean {
		return !this.#closed && !this.audio.paused && !this.#retrying && !this.#blocked;
	}

	/** The browser refused to play without a gesture, and no tap has played it since. */
	get blocked(): boolean {
		return this.#blocked;
	}

	#play(how: 'start' | 'resume' | 'retry'): Promise<boolean> {
		if (this.#closed) return Promise.resolve(false);
		this.#wantPlay = true;
		if (how !== 'retry') this.#retries = 0;
		this.#playPending = true;
		let p: Promise<void>;
		try {
			p = Promise.resolve(this.audio.play());
		} catch (e) {
			p = Promise.reject(e);
		}
		return p.then(
			() => {
				this.#playPending = false;
				if (this.#closed || !this.#wantPlay) return false;
				this.#blocked = false;
				this.#retrying = false;
				this.#pausedAt = null;
				if (how === 'retry') this.#note(`media element playing again after ${this.#retries} ${this.#retries === 1 ? 'retry' : 'retries'}`);
				this.#retries = 0;
				this.onPlaying?.();
				return true;
			},
			(e: unknown) => {
				this.#playPending = false;
				if (this.#closed) return false;
				const err = e as { name?: string; message?: string };
				const msg = err?.message ?? String(e);
				this.#note(how === 'start' ? `media element did not start: ${msg}` : how === 'resume' ? `could not resume the media element: ${msg}` : `retry ${this.#retries} failed: ${msg}`);
				if (!this.#wantPlay) return false; // paused meanwhile: nothing to try again
				if (err?.name === 'NotAllowedError') {
					this.#retrying = false;
					this.#block();
				} else if (err?.name === 'AbortError') {
					// Allowed (the browser started it), then interrupted: no tap is needed now.
					this.#blocked = false;
					if (this.#retries >= MAX_RETRIES) {
						this.#retrying = false;
						this.#note(`media element still not playing after ${this.#retries} retries`);
						this.#block();
					} else {
						this.#retrying = true; // the next canplay, loadedmetadata or segment
					}
				}
				return false;
			},
		);
	}

	#block() {
		this.#blocked = true;
		this.onBlocked?.();
	}

	/** An aborted play() goes again once the element is ready (see {@link #play}). */
	#retry(after: string) {
		if (!this.#retrying || !this.#wantPlay || this.#blocked || this.#playPending || this.#closed || this.#failed) return;
		if (!this.audio.paused) return; // playing after all
		this.#retries++;
		this.#note(`media element: retrying play (${this.#retries}) after ${after}`);
		void this.#play('retry');
	}

	/**
	 * Drop everything buffered (LMS stopped or seeked). Resolves with the timeline position the
	 * next segment should start at.
	 */
	async flush(): Promise<number> {
		this.#queue = [];
		const sb = this.#sb;
		if (!sb) return this.audio.currentTime;
		if (sb.updating) {
			try {
				sb.abort();
			} catch {
				/* not updating any more */
			}
		}
		const now = this.audio.currentTime;
		if (sb.buffered.length) {
			await new Promise<void>((resolve) => {
				sb.addEventListener('updateend', () => resolve(), { once: true });
				try {
					sb.remove(0, Infinity);
				} catch {
					resolve();
				}
			});
		}
		return now;
	}

	/**
	 * After a flush: move the play head to `t`, where the new audio starts. Removing the
	 * buffer alone doesn't make every browser drop the audio it has already decoded from the
	 * old position; a seek does, and the element then waits for the new audio.
	 */
	seek(t: number) {
		if (this.#closed || !Number.isFinite(t) || t < 0) return;
		try {
			this.audio.currentTime = t;
		} catch (e) {
			this.#note(`could not move the play head: ${(e as Error).message}`);
		}
	}

	stop() {
		this.#closed = true;
		this.#wantPlay = false;
		this.#retrying = false;
		this.#queue = [];
		this.audio.pause();
		this.audio.removeAttribute('src');
		this.audio.load();
		this.audio.remove();
		if (this.#url) URL.revokeObjectURL(this.#url);
		this.#ms = null;
		this.#sb = null;
	}
}

/** Played time spans → frames, across sample-rate changes on the timeline. */
export class Timeline {
	#spans: { at: number; rate: number }[] = [];

	/** Audio from timeline second `at` onwards is at `rate`. */
	mark(at: number, rate: number) {
		this.#spans = this.#spans.filter((s) => s.at < at);
		this.#spans.push({ at, rate });
	}

	reset(at: number, rate: number) {
		this.#spans = [{ at, rate }];
	}

	/** Frames played between timeline positions `from` and `to`. */
	frames(from: number, to: number): number {
		if (!(to > from) || !this.#spans.length) return 0;
		let total = 0;
		for (let i = 0; i < this.#spans.length; i++) {
			const s = this.#spans[i]!;
			const end = this.#spans[i + 1]?.at ?? Infinity;
			const a = Math.max(from, s.at);
			const b = Math.min(to, end);
			if (b > a) total += (b - a) * s.rate;
		}
		return Math.round(total);
	}
}
