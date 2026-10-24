import { canPlayHere, detect, hasAudioWorklet, MSE_TYPES } from '#lib/player/capabilities';
import { chooseMode } from '#lib/player/mode';

class SecureContext {
	get audioWorklet() {
		return {};
	}
}
class InsecureContext {}

test('reports managed and classic Media Source support per type', () => {
	const g = {
		ManagedMediaSource: { isTypeSupported: (t: string) => t.includes('flac') },
		MediaSource: { isTypeSupported: (t: string) => t.includes('mp4a') },
		AudioWorkletNode: class {},
		AudioContext: SecureContext,
		navigator: { userAgent: 'Test/1.0', audioSession: {} },
	};
	const c = detect(g);
	expect(c.managedMediaSource && c.mediaSource && c.audioWorklet && c.audioSession).toBe(true);
	expect(c.types).toHaveLength(MSE_TYPES.length);
	expect(c.types.find((t) => t.type === 'audio/mp4; codecs="flac"')).toMatchObject({ managed: true, classic: false });
	expect(c.types.find((t) => t.type === 'audio/mp4; codecs="mp4a.40.2"')).toMatchObject({ managed: false, classic: true });
});

test('missing APIs and throwing probes are reported as unsupported', () => {
	const c = detect({ MediaSource: { isTypeSupported: () => { throw new Error('x'); } }, navigator: {} });
	expect(c.managedMediaSource).toBe(false);
	expect(c.types.every((t) => !t.managed && !t.classic)).toBe(true);
	expect(c.audioWorklet).toBe(false);
});

// Plain http:// to anything but localhost is an insecure context: browsers withhold
// `audioWorklet` from audio contexts. Measured 2026-10-24: Firefox drops AudioWorkletNode
// too; Chromium keeps the AudioWorkletNode constructor but not `audioWorklet`.
describe('Web Audio worklets, secure and insecure contexts', () => {
	class SecureBase {
		get audioWorklet() {
			return {};
		}
	}
	class Inherited extends SecureBase {}
	const flacMse = { isTypeSupported: (t: string) => t === 'audio/mp4; codecs="flac"' };
	const shapes = {
		secure: { AudioWorkletNode: class {}, AudioContext: SecureContext, MediaSource: flacMse, Worker: class {}, WebAssembly: {} },
		chromiumInsecure: { AudioWorkletNode: class {}, AudioContext: InsecureContext, MediaSource: flacMse, Worker: class {}, WebAssembly: {} },
		firefoxInsecure: { AudioContext: InsecureContext, MediaSource: flacMse, Worker: class {}, WebAssembly: {} },
	};

	test('both present: worklets available, and the desktop default is Web Audio', () => {
		expect(hasAudioWorklet(shapes.secure)).toBe(true);
		// audioWorklet lives on BaseAudioContext.prototype in browsers: inherited counts.
		expect(hasAudioWorklet({ AudioWorkletNode: class {}, AudioContext: Inherited })).toBe(true);
		// Older Safari names the constructor webkitAudioContext.
		expect(hasAudioWorklet({ AudioWorkletNode: class {}, webkitAudioContext: SecureContext })).toBe(true);
		expect(chooseMode(detect(shapes.secure), { appleMobile: false }).mode).toBe('worklet');
		expect(canPlayHere(shapes.secure)).toBe(true);
	});

	test('constructor without audioWorklet (Chromium over plain http): no worklets, media element chosen', () => {
		const caps = detect(shapes.chromiumInsecure);
		expect(caps.audioWorklet).toBe(false);
		expect(chooseMode(caps, { appleMobile: false })).toMatchObject({ mode: 'media', source: 'classic', mime: 'audio/mp4; codecs="flac"' });
		// Even when Web Audio is asked for in Settings.
		expect(chooseMode(caps, { appleMobile: false }, 'worklet')).toMatchObject({ mode: 'media', source: 'classic' });
		expect(canPlayHere(shapes.chromiumInsecure)).toBe(true);
	});

	test('neither (Firefox over plain http): no worklets, media element chosen', () => {
		const caps = detect(shapes.firefoxInsecure);
		expect(caps.audioWorklet).toBe(false);
		expect(chooseMode(caps, { appleMobile: false })).toMatchObject({ mode: 'media', source: 'classic' });
		expect(canPlayHere(shapes.firefoxInsecure)).toBe(true);
	});

	test('no output at all: cannot play here', () => {
		expect(canPlayHere({ AudioWorkletNode: class {}, AudioContext: InsecureContext, Worker: class {}, WebAssembly: {} })).toBe(false);
		expect(canPlayHere({ ...shapes.secure, Worker: undefined })).toBe(false);
		expect(canPlayHere({ ...shapes.secure, WebAssembly: undefined })).toBe(false);
	});

	test('odd globals never throw and never claim worklets', () => {
		const hostile = new Proxy(function () {}, {
			get(t, k) {
				if (k === 'prototype') throw new Error('no');
				return Reflect.get(t, k);
			},
		});
		for (const g of [
			{},
			{ AudioWorkletNode: class {} },
			{ AudioWorkletNode: 'yes', AudioContext: SecureContext },
			{ AudioWorkletNode: class {}, AudioContext: {} },
			{ AudioWorkletNode: class {}, AudioContext: 42 },
			{ AudioWorkletNode: class {}, AudioContext: hostile },
			{ AudioWorkletNode: class {}, AudioContext: Object.assign(function () {}, { prototype: null }) },
		] as Record<string, unknown>[]) {
			expect(hasAudioWorklet(g)).toBe(false);
			expect(detect(g).audioWorklet).toBe(false);
		}
	});

	test('detection makes no audio context', () => {
		let made = 0;
		class Counting {
			constructor() {
				made++;
			}
			get audioWorklet() {
				return {};
			}
		}
		expect(detect({ AudioWorkletNode: class {}, AudioContext: Counting }).audioWorklet).toBe(true);
		canPlayHere({ AudioWorkletNode: class {}, AudioContext: Counting, Worker: class {}, WebAssembly: {} });
		expect(made).toBe(0);
	});
});
