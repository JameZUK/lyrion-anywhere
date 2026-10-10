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
