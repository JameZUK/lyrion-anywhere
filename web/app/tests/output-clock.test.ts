// When this device's audio is heard: the output clock for Web Audio, and the smoothed play
// head for the media element.
import { PlayheadSmoother, outputClock, type ClockSource } from '#lib/player/output-clock';

const ORIGIN = 1_700_000_000_000;

const ctx = (over: Partial<ClockSource> = {}): ClockSource => ({
	currentTime: 10.5,
	sampleRate: 48000,
	outputLatency: 0.04,
	baseLatency: 0.01,
	...over,
});

test('uses the output timestamp: which context time is heard when', () => {
	const c = outputClock(ctx({ getOutputTimestamp: () => ({ contextTime: 10.2, performanceTime: 5000 }) }), ORIGIN, 5010);
	expect(c).toEqual({ ctx: 10.2, at: ORIGIN + 5000, rate: 48000 });
});

// Bluetooth-sized latency where only the latency figures are known.
test('falls back to currentTime less the reported latencies', () => {
	const c = outputClock(ctx({ currentTime: 20, outputLatency: 0.25, baseLatency: 0.01 }), ORIGIN, 7000);
	expect(c!.ctx).toBeCloseTo(19.74);
	expect(c!.at).toBe(ORIGIN + 7000);
});

test('an empty, stale or broken timestamp is not used', () => {
	for (const ts of [
		{ contextTime: 0, performanceTime: 0 },
		{ contextTime: 10, performanceTime: 1000 }, // seconds away from now
		{ contextTime: NaN, performanceTime: 5000 },
		{},
	]) {
		const c = outputClock(ctx({ getOutputTimestamp: () => ts }), ORIGIN, 5000);
		expect(c!.at).toBe(ORIGIN + 5000);
		expect(c!.ctx).toBeCloseTo(10.45);
	}
	const throwing = outputClock(
		ctx({
			getOutputTimestamp: () => {
				throw new Error('not running');
			},
		}),
		ORIGIN,
		5000,
	);
	expect(throwing!.ctx).toBeCloseTo(10.45);
});

// Security/robustness: nonsense figures never produce a nonsense clock.
test('nonsense latencies are ignored and a context not yet running gives nothing', () => {
	const c = outputClock(ctx({ outputLatency: 99, baseLatency: -1 }), ORIGIN, 1);
	expect(c!.ctx).toBe(10.5);
	expect(outputClock(ctx({ currentTime: 0 }), ORIGIN, 1)).toBeNull();
	expect(outputClock(ctx({ sampleRate: 0 }), ORIGIN, 1)).toBeNull();
	expect(outputClock(ctx({ currentTime: 0.01, outputLatency: 0.05 }), ORIGIN, 1)).toBeNull();
});

test('the smoother follows a jittery play head to within a few milliseconds', () => {
	const s = new PlayheadSmoother();
	let worst = 0;
	let seed = 9;
	const rnd = () => {
		seed = (seed * 1_103_515_245 + 12_345) % 2 ** 31;
		return seed / 2 ** 31;
	};
	for (let i = 0; i < 600; i++) {
		const wall = 1000 + i * 0.1;
		const truth = 30 + i * 0.1;
		const raw = truth + (rnd() - 0.5) * 0.02; // ±10 ms of reading jitter
		const est = s.update(raw, wall);
		if (i > 30) worst = Math.max(worst, Math.abs(est - truth));
	}
	expect(worst).toBeLessThan(0.005);
});

test('the smoother starts again after a jump, and passes junk through', () => {
	const s = new PlayheadSmoother();
	s.update(10, 100);
	s.update(10.1, 100.1);
	expect(s.update(20, 100.2)).toBe(20); // a seek: taken at once
	s.reset();
	expect(s.update(5, 300)).toBe(5);
	expect(s.update(NaN, 300)).toBeNaN();
});
