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
	makeAudio?: () => HTMLAudioElement;
	ctors?: { managed?: SourceCtor; classic?: SourceCtor };
}

/** Keep this many seconds behind the play head; older audio is removed. */
const KEEP_BEHIND = 10;

export class MediaOutput {
	readonly audio: HTMLAudioElement;
	#ms: MediaSource | null = null;
	#sb: SourceBuffer | null = null;
	#url = '';
	#queue: Uint8Array[] = [];
	#closed = false;
	#note: (m: string) => void;
	/** Called when the element has played everything appended and is waiting for more. */
	onDrained: (() => void) | null = null;

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
		a.play().catch((e: Error) => this.#note(`media element did not start: ${e.message}`));
		await new Promise<void>((resolve, reject) => {
			ms.addEventListener('sourceopen', () => resolve(), { once: true });
			setTimeout(() => reject(new Error('media source did not open')), 10_000);
		});
		const sb = ms.addSourceBuffer(this.opts.mime ?? MEDIA_TYPE);
		sb.mode = 'segments';
		sb.addEventListener('updateend', () => this.#pump());
		sb.addEventListener('error', () => this.#note('media buffer error'));
		this.#sb = sb;
		ms.addEventListener('startstreaming', () => this.#note('media source: start streaming'));
		ms.addEventListener('endstreaming', () => this.#note('media source: end streaming'));
		a.addEventListener('error', () => this.#note(`media element error ${a.error?.code ?? ''}: ${a.error?.message ?? ''}`));
		ms.addEventListener('sourceclose', () => this.#note('media source closed'));
		a.addEventListener('waiting', () => {
			if (this.ahead < 0.1) this.onDrained?.();
		});
		this.#pump();
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

	pause() {
		this.audio.pause();
	}

	resume() {
		this.audio.play().catch((e: Error) => this.#note(`could not resume the media element: ${e.message}`));
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

	stop() {
		this.#closed = true;
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
