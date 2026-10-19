// Media-element output basics, against a fake Media Source.
import { MediaOutput, Timeline, describeBuffered, describeMediaEvent } from '$lib/player/media-output';

class FakeBuffered {
	constructor(public spans: [number, number][] = []) {}
	get length() {
		return this.spans.length;
	}
	start(i: number) {
		return this.spans[i]![0];
	}
	end(i: number) {
		return this.spans[i]![1];
	}
}

class FakeSourceBuffer extends EventTarget {
	static last: FakeSourceBuffer | null = null;
	mode = '';
	updating = false;
	buffered = new FakeBuffered();
	appended: Uint8Array[] = [];
	/** Every appendBuffer call, refused or not. */
	attempts = 0;
	/** While this says so, appendBuffer refuses for want of space (as a full browser buffer). */
	full: (() => boolean) | null = null;
	constructor() {
		super();
		FakeSourceBuffer.last = this;
	}
	appendBuffer(b: Uint8Array) {
		if (this.updating) throw new DOMException('busy', 'InvalidStateError');
		this.attempts++;
		if (this.full?.()) throw new DOMException('The quota has been exceeded.', 'QuotaExceededError');
		this.updating = true;
		this.appended.push(b);
		setTimeout(() => {
			this.updating = false;
			this.dispatchEvent(new Event('updateend'));
		}, 0);
	}
	removed: [number, number][] = [];
	/** A removal that frees nothing: the buffered ranges stay as they were. */
	keepOnRemove = false;
	remove(start: number, end: number) {
		if (this.updating) throw new DOMException('busy', 'InvalidStateError');
		this.updating = true;
		this.removed.push([start, end]);
		setTimeout(() => {
			if (this.keepOnRemove) {
				this.updating = false;
				this.dispatchEvent(new Event('updateend'));
				return;
			}
			const kept: [number, number][] = [];
			for (const [a, b] of this.buffered.spans) {
				if (a < start) kept.push([a, Math.min(b, start)]);
				if (b > end) kept.push([Math.max(a, end), b]);
			}
			this.buffered = new FakeBuffered(kept);
			this.updating = false;
			this.dispatchEvent(new Event('updateend'));
		}, 0);
	}
	abort() {}
}

class FakeMediaSource extends EventTarget {
	types: string[] = [];
	constructor() {
		super();
		setTimeout(() => this.dispatchEvent(new Event('sourceopen')), 0);
	}
	addSourceBuffer(type: string) {
		this.types.push(type);
		return new FakeSourceBuffer();
	}
}

function fakeAudio() {
	const a = document.createElement('audio');
	a.play = vi.fn(async () => {});
	a.pause = vi.fn();
	a.load = vi.fn();
	return a;
}

beforeAll(() => {
	URL.createObjectURL = () => 'blob:ms';
	URL.revokeObjectURL = () => {};
});

const ctors = { classic: FakeMediaSource as unknown as new () => MediaSource };
const settle = async () => {
	for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

test('starts the element, opens the source, and appends segments in order, one at a time', async () => {
	const out = new MediaOutput({ source: 'classic', mime: 'audio/mp4; codecs="flac"', makeAudio: fakeAudio, ctors });
	await out.start();
	expect(out.audio.play).toHaveBeenCalled();
	const sb = FakeSourceBuffer.last!;
	const segs = [1, 2, 3, 4].map((n) => new Uint8Array([n]));
	for (const s of segs) out.append(s); // would throw in the fake if appended while updating
	await settle();
	expect(sb.appended.map((b) => b[0])).toEqual([1, 2, 3, 4]);
	out.stop();
});

test('nothing is appended after stop, and the element leaves the page', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	expect(out.audio.isConnected).toBe(true);
	out.stop();
	out.append(new Uint8Array([9]));
	await settle();
	expect(FakeSourceBuffer.last!.appended).toHaveLength(0);
	expect(out.audio.isConnected).toBe(false);
});

test('a missing Media Source is an error, not a hang', async () => {
	const out = new MediaOutput({ source: 'managed', makeAudio: fakeAudio, ctors: {} });
	await expect(out.start()).rejects.toThrow(/not available/);
});

test('a buffer error reports the failure once', async () => {
	const onFail = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, onFail });
	await out.start();
	FakeSourceBuffer.last!.dispatchEvent(new Event('error'));
	FakeSourceBuffer.last!.dispatchEvent(new Event('error'));
	expect(onFail).toHaveBeenCalledTimes(1);
	out.stop();
});

test('flush empties the buffer and reports where to carry on', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	FakeSourceBuffer.last!.buffered = new FakeBuffered([[0, 5]]);
	expect(out.bufferedEnd).toBe(5);
	const at = await out.flush();
	expect(at).toBe(0);
	expect(out.bufferedEnd).toBe(0);
	out.stop();
});

// After a flush the play head is moved to where the new audio starts, so the browser drops
// audio it had already decoded from the old position.
test('seek moves the play head; nonsense positions and a stopped output are ignored', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	let set: number[] = [];
	Object.defineProperty(out.audio, 'currentTime', {
		configurable: true,
		get: () => set[set.length - 1] ?? 0,
		set: (v: number) => void set.push(v),
	});
	out.seek(12.5);
	out.seek(NaN);
	out.seek(-1);
	out.seek(Infinity);
	expect(set).toEqual([12.5]);
	out.stop();
	set = [];
	out.seek(3);
	expect(set).toEqual([]);
});

test('a play head that refuses to move is noted, not thrown', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	Object.defineProperty(out.audio, 'currentTime', {
		configurable: true,
		get: () => 0,
		set: () => {
			throw new Error('not seekable');
		},
	});
	expect(() => out.seek(1)).not.toThrow();
	expect(note).toHaveBeenCalledWith(expect.stringMatching(/not seekable/));
	out.stop();
});

test('the timeline counts frames across rate changes', () => {
	const t = new Timeline();
	expect(t.frames(0, 1)).toBe(0); // no spans yet
	t.mark(0, 44100);
	t.mark(10, 96000);
	expect(t.frames(0, 1)).toBe(44100);
	expect(t.frames(9, 11)).toBe(44100 + 96000);
	expect(t.frames(5, 5)).toBe(0);
	expect(t.frames(6, 5)).toBe(0);
	expect(t.frames(NaN, 5)).toBe(0);
	t.reset(0, 48000);
	expect(t.frames(0, 2)).toBe(96000);
});

/** Control what the fake element reports: playing or paused, and where its play head is. */
function control(out: MediaOutput, state: { paused: boolean; t: number }) {
	Object.defineProperty(out.audio, 'paused', { configurable: true, get: () => state.paused });
	Object.defineProperty(out.audio, 'currentTime', {
		configurable: true,
		get: () => state.t,
		set: (v: number) => void (state.t = v),
	});
}

// Regression: a stall that was not the end (the element paused, its play head stuck just
// before the new audio after a seek, or audio still being appended) was reported as "played
// everything", so the worker counted audio still to come as played. Near the end of a track
// that told LMS the next track had started.
test('only a play head at the end of everything appended counts as drained', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	const drained = vi.fn();
	out.onDrained = drained;
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 0 };
	control(out, s);
	const waiting = () => out.audio.dispatchEvent(new Event('waiting'));

	// Nothing appended yet (play() before the first segment): not the end.
	waiting();
	expect(out.atEnd).toBe(false);

	// The play head a hair before the new audio (a seek that landed just short): not the end,
	// and the play head is moved onto the audio.
	sb.buffered = new FakeBuffered([[100.0001, 108]]);
	s.t = 100;
	waiting();
	expect(drained).not.toHaveBeenCalled();
	expect(s.t).toBe(100.0001);

	// Paused at the end of the audio: not a drain either.
	s.t = 107.95;
	s.paused = true;
	waiting();
	expect(drained).not.toHaveBeenCalled();
	s.paused = false;

	// Audio still waiting to be appended: not the end.
	sb.updating = true;
	out.append(new Uint8Array([1]));
	sb.updating = false;
	waiting();
	expect(drained).not.toHaveBeenCalled();
	out.append(new Uint8Array([2])); // pumps the queue
	await settle();

	// Played up to the end of everything appended: drained.
	waiting();
	expect(drained).toHaveBeenCalledTimes(1);
	out.stop();
});

// Regression: when the system paused the element (a call, another app's audio), nothing
// noticed: LMS and the player carried on as if playing while nothing was heard.
test('a pause the output did not ask for is reported once; its own pauses are not', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	const external = vi.fn();
	out.onExternalPause = external;
	await out.start();
	const pauseEvent = () => out.audio.dispatchEvent(new Event('pause'));

	out.pause(); // LMS paused: our own
	pauseEvent();
	expect(external).not.toHaveBeenCalled();

	out.resume();
	pauseEvent(); // the system's
	pauseEvent(); // reported once, until we play again
	expect(external).toHaveBeenCalledTimes(1);

	out.resume();
	out.stop();
	pauseEvent(); // stopping is ours too
	expect(external).toHaveBeenCalledTimes(1);
});

test('a play head far from any audio is left alone; one inside audio is not stuck', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 10 };
	control(out, s);
	sb.buffered = new FakeBuffered([[20, 30]]);
	expect(out.unstick()).toBe(false); // 10 s away: not a sliver of a gap
	expect(s.t).toBe(10);
	s.t = 25;
	expect(out.unstick()).toBe(false);
	expect(s.t).toBe(25);
	s.t = 19.8;
	s.paused = true;
	expect(out.unstick()).toBe(false); // paused: nothing to do
	s.paused = false;
	expect(out.unstick()).toBe(true);
	expect(s.t).toBe(20);
	out.stop();
});

// Regression: after LMS's `strm f` (the next track deleted or moved), the element played the
// segments of the dropped track it already had.
test('a trim removes the audio from the cut, after the segments queued before it', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 3 };
	control(out, s);
	sb.buffered = new FakeBuffered([[0, 8]]);
	out.append(new Uint8Array([1]));
	out.trimFrom(5.5);
	out.append(new Uint8Array([2])); // the kept audio again, from the cut
	await settle();
	expect(sb.appended.map((b) => b[0])).toEqual([1, 2]);
	expect(sb.removed).toEqual([[5.5, Infinity]]);
	expect(out.bufferedEnd).toBe(5.5);
	expect(s.t).toBe(3); // the play head is left where it is
	out.stop();
});

test('a trim behind the play head moves it back to the cut; one past the end does nothing', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 6 };
	control(out, s);
	sb.buffered = new FakeBuffered([[0, 8]]);
	out.trimFrom(5.5);
	await settle();
	expect(s.t).toBe(5.5);
	out.trimFrom(9);
	await settle();
	expect(sb.removed).toEqual([[5.5, Infinity]]);
	out.append(new Uint8Array([3]));
	await settle();
	expect(sb.appended.map((b) => b[0])).toEqual([3]);
	out.stop();
});

test('nonsense trims are ignored, and a refused removal is noted, not thrown', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	sb.buffered = new FakeBuffered([[0, 8]]);
	for (const bad of [NaN, -1, Infinity]) out.trimFrom(bad);
	await settle();
	expect(sb.removed).toEqual([]);
	sb.remove = () => {
		throw new DOMException('gone', 'InvalidStateError');
	};
	out.trimFrom(2);
	out.append(new Uint8Array([4]));
	await settle();
	expect(note).toHaveBeenCalledWith(expect.stringMatching(/gone/));
	expect(sb.appended.map((b) => b[0])).toEqual([4]);
	out.stop();
	out.trimFrom(1); // after stop: nothing
});

/**
 * An element whose play() behaves as a browser's: `results` says how each call ends ('ok', or
 * the error name to reject with). An AbortError comes from a load or source change while the
 * play was pending: the element is paused again afterwards, without a pause event.
 */
function playable(out: MediaOutput, results: string[]) {
	const state = { paused: true, t: 0 };
	control(out, state);
	const calls: string[] = [];
	out.audio.play = vi.fn(async () => {
		const r = results.shift() ?? 'ok';
		calls.push(r);
		if (r === 'NotAllowedError') throw new DOMException('not allowed', r);
		state.paused = false;
		if (r !== 'ok') {
			await Promise.resolve();
			state.paused = true;
			throw new DOMException('The operation was aborted.', r);
		}
	});
	return { state, calls };
}

// Live, iPadOS PWA: the tap's play() was aborted by a stream restart at the same moment and
// nothing tried again, so the element stayed paused while everything else carried on as if
// it were playing.
test('an aborted play() is tried again once the element is ready (canplay)', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const { state, calls } = playable(out, ['AbortError', 'ok']);
	const onPlaying = vi.fn();
	out.onPlaying = onPlaying;
	await expect(out.resume()).resolves.toBe(false);
	expect(state.paused).toBe(true);
	out.audio.dispatchEvent(new Event('canplay'));
	await settle();
	expect(calls).toEqual(['AbortError', 'ok']);
	expect(state.paused).toBe(false);
	expect(out.playing).toBe(true);
	expect(onPlaying).toHaveBeenCalledTimes(1);
	expect(note).toHaveBeenCalledWith(expect.stringMatching(/retrying play \(1\) after canplay/));
	out.stop();
});

test('an aborted play() is retried at each segment appended until it plays', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const { state, calls } = playable(out, ['AbortError', 'AbortError', 'AbortError', 'ok']);
	await out.resume();
	out.append(new Uint8Array([1]));
	await settle();
	out.append(new Uint8Array([2]));
	await settle();
	expect(state.paused).toBe(true);
	out.append(new Uint8Array([3]));
	await settle();
	expect(calls).toEqual(['AbortError', 'AbortError', 'AbortError', 'ok']);
	expect(state.paused).toBe(false);
	// Playing: further segments call play() no more.
	out.append(new Uint8Array([4]));
	await settle();
	expect(calls).toHaveLength(4);
	expect(note.mock.calls.filter(([m]) => /retrying play/.test(m))).toHaveLength(3);
	out.stop();
});

test('no retry after a pause or a stop', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const { calls } = playable(out, ['AbortError']);
	await out.resume();
	out.pause();
	out.audio.dispatchEvent(new Event('canplay'));
	out.append(new Uint8Array([1]));
	await settle();
	expect(calls).toEqual(['AbortError']);
	out.stop();
});

test('a refused play() (no gesture) reports the block and is not retried by itself', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const { calls } = playable(out, ['NotAllowedError', 'ok']);
	const onBlocked = vi.fn();
	out.onBlocked = onBlocked;
	await expect(out.resume()).resolves.toBe(false);
	expect(onBlocked).toHaveBeenCalledTimes(1);
	expect(out.blocked).toBe(true);
	expect(out.playing).toBe(false);
	out.audio.dispatchEvent(new Event('canplay'));
	out.append(new Uint8Array([1]));
	await settle();
	expect(calls).toEqual(['NotAllowedError']);
	// The tap: plays.
	await expect(out.resume()).resolves.toBe(true);
	expect(out.blocked).toBe(false);
	expect(out.playing).toBe(true);
	out.stop();
});

test('retries that keep failing end in the blocked state, not silence', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	playable(out, Array(50).fill('AbortError'));
	const onBlocked = vi.fn();
	out.onBlocked = onBlocked;
	await out.resume();
	for (let i = 0; i < 20; i++) {
		out.audio.dispatchEvent(new Event('canplay'));
		await settle();
	}
	expect(onBlocked).toHaveBeenCalledTimes(1);
	expect(out.blocked).toBe(true);
	out.stop();
});

test('pausedAt: where it stopped playing; null while playing or before it ever played', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const { state } = playable(out, ['NotAllowedError', 'ok']);
	await out.resume();
	expect(out.pausedAt).toBeNull(); // never played
	await out.resume();
	state.t = 4;
	out.pause();
	state.paused = true;
	state.t = 9; // moved while paused: not where it stopped
	out.audio.dispatchEvent(new Event('pause'));
	expect(out.pausedAt).toBe(4);
	await out.resume();
	expect(out.pausedAt).toBeNull();
	out.stop();
});

test('a deferred start opens the source but does not call play() until asked', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, deferPlay: true });
	await out.start();
	expect(out.audio.play).not.toHaveBeenCalled();
	expect(out.blocked).toBe(true);
	out.append(new Uint8Array([1]));
	await settle();
	expect(FakeSourceBuffer.last!.appended).toHaveLength(1);
	expect(out.audio.play).not.toHaveBeenCalled();
	out.stop();
});

/** Seconds of audio the fake buffer holds. */
const held = (sb: FakeSourceBuffer) => sb.buffered.spans.reduce((n, [a, b]) => n + (b - a), 0);

// Issue #12 (iPadOS, Original/FLAC): after a scrub the media buffer filled ("media buffer full;
// waiting") and nothing ever made room or tried again: no further append arrived while the
// worker was held back, so the element ran dry, the worker stopped acknowledging, and the
// gateway ended the stream a minute later. The audio behind the play head is removed at once
// and the queue carries on by itself.
test('a full buffer removes the audio behind the play head and resumes without another append', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 100 };
	control(out, s);
	// Old audio from before the scrub sits behind the play head; the buffer takes 12 s at most.
	sb.buffered = new FakeBuffered([[92, 106]]);
	sb.full = () => held(sb) > 12;
	out.append(new Uint8Array([7]));
	await settle();
	expect(sb.removed).toHaveLength(1);
	const [from, to] = sb.removed[0]!;
	expect(from).toBe(0);
	expect(to).toBeLessThanOrEqual(s.t); // only what has played
	expect(to).toBeGreaterThan(92 + 0.5);
	expect(sb.appended.map((b) => b[0])).toEqual([7]); // no further append() was needed
	// Both lines say where the play head and the audio were, and carry a kind for the limiter.
	expect(note).toHaveBeenCalledWith('media buffer full at 100.0 s (segment 1 KB), buffered 92.0–106.0 s; waiting', 'media:full');
	expect(note).toHaveBeenCalledWith('media buffer trimmed; resuming at 100.0 s, buffered 99.0–106.0 s', 'media:resume');
	expect(s.t).toBe(100); // the play head is left alone
	out.stop();
});

// Issue #12 follow-up: on an iPad the first track of a session stayed at 0.0 s with ten
// seconds buffered, after play() on a source that was still empty. A play head inside
// buffered audio that has not moved is kicked (a seek to the same place), at most once per
// 5 s; gaps are unstick()'s job, and a paused or nearly drained element is left alone.
test('a play head stuck inside buffered audio is kicked, rate-limited, never when paused', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 0 };
	let seeks = 0;
	Object.defineProperty(out.audio, 'paused', { configurable: true, get: () => s.paused });
	Object.defineProperty(out.audio, 'currentTime', {
		configurable: true,
		get: () => s.t,
		set: (v: number) => {
			seeks++;
			s.t = v;
		},
	});
	sb.buffered = new FakeBuffered([[0, 10.3]]);
	expect(out.kick(1_000)).toBe(true);
	expect(seeks).toBe(1);
	expect(s.t).toBe(0); // the same place: nothing is skipped
	expect(note).toHaveBeenCalledWith('media: kicked the play head at 0.0 s, buffered 0.0–10.3 s', 'media:kick');
	expect(out.kick(3_000)).toBe(false); // not again so soon
	expect(out.kick(6_001)).toBe(true);
	expect(seeks).toBe(2);
	s.t = 10; // less than half a second ahead: nearly drained, not stuck
	expect(out.kick(20_000)).toBe(false);
	s.t = 2;
	s.paused = true;
	expect(out.kick(30_000)).toBe(false);
	s.paused = false;
	sb.buffered = new FakeBuffered([[2.2, 10]]); // a gap ahead: unstick()'s case, not a kick
	expect(out.kick(40_000)).toBe(false);
	expect(seeks).toBe(2);
	out.stop();
});

// Issue #12 follow-up: hi-res audio must not sit on the browser's media buffer quota (about
// 5.5 MB for audio on an iPad). The worker, knowing the stream's data rate, asks the output
// to keep less played audio behind the play head than the 10 s default.
test('the keep-behind set by the worker trims played audio sooner, within sane bounds', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 30 };
	control(out, s);
	expect(out.keepBehind).toBe(10);
	out.setKeepBehind(1.8);
	expect(out.keepBehind).toBe(1.8);
	sb.buffered = new FakeBuffered([[0, 40]]);
	out.append(new Uint8Array([3]));
	await settle();
	expect(sb.removed).toHaveLength(1);
	expect(sb.removed[0]![0]).toBe(0);
	expect(sb.removed[0]![1]).toBeCloseTo(28.2, 5); // 30 − 1.8, not 30 − 10
	await settle();
	expect(sb.appended.map((b) => b[0])).toEqual([3]); // the segment follows the trim
	// Clamped: never below the full-buffer keep-behind, never above the default; junk ignored.
	out.setKeepBehind(0.2);
	expect(out.keepBehind).toBe(1);
	out.setKeepBehind(50);
	expect(out.keepBehind).toBe(10);
	out.setKeepBehind(NaN);
	expect(out.keepBehind).toBe(10);
	out.stop();
});

test('a full buffer with nothing played to remove tries again as the play head moves, not in a loop', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 100 };
	control(out, s);
	sb.buffered = new FakeBuffered([[99.8, 112]]); // all of it still to play
	sb.full = () => held(sb) > 10;
	out.append(new Uint8Array([1]));
	await settle();
	expect(sb.removed).toEqual([]);
	expect(sb.appended).toHaveLength(0);
	expect(sb.attempts).toBe(1); // waits: no spinning on the refusal
	expect(note.mock.calls.filter(([, k]) => k === 'media:full')).toHaveLength(1);
	// With nothing behind the play head to remove, the log says so (rate-limited too).
	expect(note).toHaveBeenCalledWith('media buffer full with nothing to trim behind 100.0 s, buffered 99.8–112.0 s; waiting for the play head', 'media:full-stuck');
	// Played on a little: still nothing worth removing; one try per event, at most.
	s.t = 100.2;
	out.audio.dispatchEvent(new Event('timeupdate'));
	await settle();
	expect(sb.attempts).toBe(2);
	expect(sb.removed).toEqual([]);
	// Each refusal is noted by kind; the page's limiter keeps the log to one line per 5 s.
	expect(note.mock.calls.filter(([, k]) => k === 'media:full')).toHaveLength(2);
	// Played on further: the played audio goes, and the segment is appended.
	s.t = 104;
	out.audio.dispatchEvent(new Event('timeupdate'));
	await settle();
	expect(sb.removed).toHaveLength(1);
	expect(sb.removed[0]![1]).toBeLessThanOrEqual(104);
	expect(sb.appended.map((b) => b[0])).toEqual([1]);
	expect(note.mock.calls.filter(([, k]) => k === 'media:full')).toHaveLength(3);
	expect(note.mock.calls.filter(([, k]) => k === 'media:resume')).toEqual([['media buffer trimmed; resuming at 104.0 s, buffered 103.0–112.0 s', 'media:resume']]);
	// Recovered: timeupdates no longer append anything.
	const before = sb.attempts;
	out.audio.dispatchEvent(new Event('timeupdate'));
	out.audio.dispatchEvent(new Event('progress'));
	await settle();
	expect(sb.attempts).toBe(before);
	out.stop();
});

test('a buffer that stays full after a removal does not remove and retry in a loop', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: false, t: 100 };
	control(out, s);
	sb.buffered = new FakeBuffered([[91, 110]]);
	sb.full = () => true; // the browser never makes room
	sb.keepOnRemove = true; // and reports the removed audio still there
	out.append(new Uint8Array([1]));
	await settle();
	await settle();
	expect(sb.removed).toHaveLength(1);
	expect(sb.attempts).toBeLessThanOrEqual(2);
	// Only as the play head moves on is it tried again.
	s.t = 101;
	out.audio.dispatchEvent(new Event('timeupdate'));
	await settle();
	expect(sb.removed.length).toBeLessThanOrEqual(2);
	expect(sb.attempts).toBeLessThanOrEqual(4);
	out.stop();
});

test('a flush forgets a full buffer: the next segment is appended as usual', async () => {
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	const s = { paused: true, t: 50 };
	control(out, s);
	sb.buffered = new FakeBuffered([[49.9, 60]]);
	sb.full = () => held(sb) > 5;
	out.append(new Uint8Array([1]));
	await settle();
	expect(sb.appended).toHaveLength(0);
	await out.flush();
	expect(out.bufferedEnd).toBe(0);
	out.append(new Uint8Array([2]));
	await settle();
	expect(sb.appended.map((b) => b[0])).toEqual([2]); // the pre-flush segment is gone
	out.stop();
});

// Issue #12 diagnostics: each element event that bears on playing is noted with the play head
// and the buffered audio nearest it; all but the state changes are rate-limited by kind.
test('element events are noted with the play head and the buffered ranges around it', async () => {
	const note = vi.fn();
	const out = new MediaOutput({ source: 'classic', makeAudio: fakeAudio, ctors, note });
	await out.start();
	const sb = FakeSourceBuffer.last!;
	sb.buffered = new FakeBuffered([
		[0, 10.04],
		[30, 40],
		[60.94, 75.42],
		[90, 100],
	]);
	out.audio.currentTime = 61.23;
	note.mockClear();
	for (const type of ['play', 'pause', 'playing', 'waiting', 'stalled', 'seeking', 'seeked', 'ended', 'error']) out.audio.dispatchEvent(new Event(type));
	const lines = note.mock.calls.map(([m, kind]) => [m, kind]);
	const where = 'at 61.2 s, buffered 30.0–40.0, 60.9–75.4 s';
	for (const type of ['waiting', 'stalled', 'seeking', 'seeked', 'error']) expect(lines).toContainEqual([`media: ${type} ${where}`, `media:${type}`]);
	// State changes carry no kind: never left out.
	for (const type of ['play', 'pause', 'playing', 'ended']) expect(lines).toContainEqual([`media: ${type} ${where}`, undefined]);
	// Nothing after stop.
	out.stop();
	note.mockClear();
	out.audio.dispatchEvent(new Event('waiting'));
	expect(note).not.toHaveBeenCalled();
});

test('the buffered ranges: the two nearest the play head, in order, and junk is harmless', () => {
	const b = (spans: [number, number][]) => new FakeBuffered(spans);
	expect(describeBuffered(b([]), 5)).toBe('nothing buffered');
	expect(describeBuffered(null, 5)).toBe('nothing buffered');
	expect(describeBuffered(b([[0, 12.06]]), 5)).toBe('buffered 0.0–12.1 s');
	const four = b([
		[0, 1],
		[2, 3],
		[10, 20],
		[50, 60],
	]);
	expect(describeBuffered(four, 15)).toBe('buffered 2.0–3.0, 10.0–20.0 s');
	expect(describeBuffered(four, 45)).toBe('buffered 10.0–20.0, 50.0–60.0 s');
	const junk = b([
		[NaN, 1],
		[5, Infinity],
		[3, 2],
		[7, 8],
	]);
	expect(describeBuffered(junk, 0)).toBe('buffered 7.0–8.0 s');
	const throwing = {
		length: 2,
		start: () => {
			throw new DOMException('gone', 'InvalidStateError');
		},
		end: () => 0,
	};
	expect(describeBuffered(throwing, 0)).toBe('nothing buffered');
	expect(describeMediaEvent('waiting', NaN, b([[0, 1]]))).toBe('media: waiting at 0.0 s, buffered 0.0–1.0 s');
});
