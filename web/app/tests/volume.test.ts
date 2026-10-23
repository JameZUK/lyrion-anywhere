import { describe, expect, it, vi } from 'vitest';
import {
	DEVICE_BUTTONS_NOTE,
	ELEMENT_RAMP_MS,
	ElementVolume,
	belowFull,
	canFollow,
	clampGain,
	cleanGains,
	detectMediaVolume,
	elementVolume,
	loadVolumeSetting,
	outputGains,
	planVolume,
	rampSteps,
	saveVolumeSetting,
	type VolumeAbility,
} from '#lib/player/volume';

const desktop: VolumeAbility = { mode: 'worklet', mediaVolume: true };
const android: VolumeAbility = { mode: 'media', mediaVolume: true };
const ipad: VolumeAbility = { mode: 'media', mediaVolume: false };
const ipadWebAudio: VolumeAbility = { mode: 'worklet', mediaVolume: false };

describe('volume plan', () => {
	it('automatic follows LMS wherever the output can be set', () => {
		expect(planVolume(desktop, 'auto')).toMatchObject({ follow: true, apply: 'gain' });
		expect(planVolume(android, 'auto')).toMatchObject({ follow: true, apply: 'element' });
		// Web Audio can always apply a gain, even where the element's volume is read-only.
		expect(planVolume(ipadWebAudio, 'auto')).toMatchObject({ follow: true, apply: 'gain' });
	});

	it('automatic falls back to the device buttons where the element volume is read-only', () => {
		const p = planVolume(ipad, 'auto');
		expect(p).toMatchObject({ follow: false, apply: 'none' });
		expect(p.reason).toMatch(/volume buttons/);
	});

	it('"device buttons only" never applies LMS volume', () => {
		for (const a of [desktop, android, ipad, ipadWebAudio]) {
			expect(planVolume(a, 'device')).toMatchObject({ follow: false, apply: 'none' });
		}
	});

	it('"follow LMS volume" is honoured only where it works, with the reason otherwise', () => {
		expect(planVolume(android, 'follow')).toMatchObject({ follow: true, apply: 'element' });
		expect(planVolume(desktop, 'follow')).toMatchObject({ follow: true, apply: 'gain' });
		const p = planVolume(ipad, 'follow');
		expect(p.follow).toBe(false);
		expect(p.reason).toMatch(/isn't possible here/);
		const c = canFollow(ipad);
		expect(c.ok).toBe(false);
		if (!c.ok) expect(c.why).toMatch(/iPhone and iPad/);
		expect(canFollow(android).ok).toBe(true);
	});

	it('uses British English for the note', () => {
		expect(DEVICE_BUTTONS_NOTE).toBe("Use this device's volume buttons");
	});
});

describe('gains', () => {
	it('applies LMS gains as given when following, full scale otherwise', () => {
		const follow = planVolume(desktop, 'auto');
		const buttons = planVolume(desktop, 'device');
		expect(outputGains(follow, [0.25, 0.5])).toEqual([0.25, 0.5]);
		expect(outputGains(buttons, [0.25, 0.5])).toEqual([1, 1]);
		expect(outputGains(null, [0.25, 0.5])).toEqual([1, 1]);
	});

	// Robustness: values from the worker are checked; nothing can amplify or go NaN.
	it('cleans hostile or malformed gains', () => {
		expect(cleanGains([0.5, 0.25])).toEqual([0.5, 0.25]);
		expect(cleanGains([0.5])).toEqual([0.5, 0.5]);
		expect(cleanGains([NaN, Infinity])).toEqual([1, 1]);
		expect(cleanGains([-1, 7])).toEqual([0, 1]);
		expect(cleanGains(['0.5', null])).toEqual([1, 1]);
		expect(cleanGains('loud')).toEqual([1, 1]);
		expect(cleanGains([])).toEqual([1, 1]);
		expect(cleanGains(undefined)).toEqual([1, 1]);
		expect(clampGain(Number.MAX_VALUE)).toBe(1);
	});

	it('an element has one volume: the louder side wins', () => {
		expect(elementVolume([0.2, 0.6])).toBe(0.6);
		expect(elementVolume([1, 1])).toBe(1);
	});

	it('tells full scale from a lowered volume', () => {
		expect(belowFull([1, 1])).toBe(false);
		expect(belowFull([0.9995, 1])).toBe(false);
		expect(belowFull([0.5, 1])).toBe(true);
	});
});

describe('ramps', () => {
	it('ends exactly on the target, in even steps', () => {
		expect(rampSteps(1, 0.5, 5)).toEqual([0.9, 0.8, 0.7, 0.6, 0.5]);
		expect(rampSteps(0, 1, 0)).toEqual([1]);
		const r = rampSteps(0.1, 0.7, 3);
		expect(r[2]).toBe(0.7);
	});

	it('sets the element volume at once, then ramps to the target', () => {
		vi.useFakeTimers();
		try {
			const el = { volume: 1 };
			const v = new ElementVolume(el);
			v.set(0.4);
			expect(el.volume).toBeLessThan(1);
			expect(el.volume).toBeGreaterThan(0.4);
			vi.advanceTimersByTime(ELEMENT_RAMP_MS + 10);
			expect(el.volume).toBe(0.4);
		} finally {
			vi.useRealTimers();
		}
	});

	it('a new value replaces a ramp still running', () => {
		vi.useFakeTimers();
		try {
			const el = { volume: 1 };
			const v = new ElementVolume(el);
			v.set(0);
			vi.advanceTimersByTime(10);
			v.set(0.8);
			vi.advanceTimersByTime(ELEMENT_RAMP_MS + 10);
			expect(el.volume).toBe(0.8);
			v.set(2); // clamped
			vi.advanceTimersByTime(ELEMENT_RAMP_MS + 10);
			expect(el.volume).toBe(1);
			v.cancel();
		} finally {
			vi.useRealTimers();
		}
	});

	it('copes with an element whose volume cannot be written', () => {
		const el = {
			get volume() {
				return 1;
			},
			set volume(_v: number) {
				throw new Error('read-only');
			},
		};
		expect(() => new ElementVolume(el).set(0.3)).not.toThrow();
	});
});

describe('detection and storage', () => {
	it('detects a settable element volume by feature test', () => {
		expect(detectMediaVolume(() => ({ volume: 1 }) as HTMLMediaElement)).toBe(true);
		// iPhone/iPad: the assignment is ignored and the volume reads 1.
		const ios = {
			get volume() {
				return 1;
			},
			set volume(_v: number) {},
		};
		expect(detectMediaVolume(() => ios as unknown as HTMLMediaElement)).toBe(false);
		expect(
			detectMediaVolume(() => {
				throw new Error('no Audio');
			}),
		).toBe(false);
	});

	it('keeps the setting per browser, and survives storage that throws', () => {
		const m = new Map<string, string>();
		const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
		expect(loadVolumeSetting(store)).toBe('auto');
		saveVolumeSetting('device', store);
		expect(loadVolumeSetting(store)).toBe('device');
		saveVolumeSetting('auto', store);
		expect(m.size).toBe(0);
		m.set('anywhere.device.volume', 'loud');
		expect(loadVolumeSetting(store)).toBe('auto');
		const broken = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('blocked');
			},
			removeItem: () => {
				throw new Error('blocked');
			},
		};
		expect(loadVolumeSetting(broken)).toBe('auto');
		expect(() => saveVolumeSetting('follow', broken)).not.toThrow();
	});
});
