// Media-element output basics, against a fake Media Source.
import { MediaOutput, Timeline } from '$lib/player/media-output';

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
	constructor() {
		super();
		FakeSourceBuffer.last = this;
	}
	appendBuffer(b: Uint8Array) {
		if (this.updating) throw new DOMException('busy', 'InvalidStateError');
		this.updating = true;
		this.appended.push(b);
		setTimeout(() => {
			this.updating = false;
			this.dispatchEvent(new Event('updateend'));
		}, 0);
	}
	remove() {
		setTimeout(() => {
			this.buffered = new FakeBuffered();
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
