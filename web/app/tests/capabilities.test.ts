import { detect, MSE_TYPES } from '$lib/player/capabilities';

test('reports managed and classic Media Source support per type', () => {
	const g = {
		ManagedMediaSource: { isTypeSupported: (t: string) => t.includes('flac') },
		MediaSource: { isTypeSupported: (t: string) => t.includes('mp4a') },
		AudioWorkletNode: class {},
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
