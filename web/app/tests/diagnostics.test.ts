import { Reporter, buildReport } from '$lib/player/diagnostics';

afterEach(() => vi.useRealTimers());

const input = {
	capabilities: { managedMediaSource: true, mediaSource: true, audioWorklet: true, audioSession: true, types: [], userAgent: 'UA' },
	environment: { appleMobile: true },
	preference: 'auto' as const,
	planned: { mode: 'media' as const, source: 'managed' as const, mime: 'audio/mp4; codecs="fLaC"', reason: 'iPad' },
	active: null,
	device: { state: 'connected', mac: '02:4c:41:00:00:09', name: 'Safari on iOS', detail: { output: 'Playing' } },
	log: Array.from({ length: 200 }, (_, i) => `line ${i}`),
	page: { visibility: 'visible', standalone: false },
};

test('reports carry the playback facts and a bounded log', () => {
	const r = buildReport(input, new Date('2026-10-10T16:00:00Z'));
	expect(r.playback.planned.mime).toBe('audio/mp4; codecs="fLaC"');
	expect(r.userAgent).toBe('UA');
	expect(r.log).toHaveLength(60);
	expect(r.at).toBe('2026-10-10T16:00:00.000Z');
	expect(JSON.stringify(r).length).toBeLessThan(32 * 1024);
});

test('reports are batched, with urgent ones sent at once', async () => {
	vi.useFakeTimers();
	const sent: boolean[] = [];
	let t = 0;
	const r = new Reporter(async (_b, urgent) => void sent.push(urgent), () => ({}), 15_000, () => t);
	r.schedule();
	r.schedule();
	r.schedule();
	await vi.advanceTimersByTimeAsync(0);
	expect(sent).toEqual([false]);
	t = 1000;
	r.schedule();
	await vi.advanceTimersByTimeAsync(5000);
	expect(sent).toHaveLength(1); // waits out the gap
	r.urgent();
	await vi.advanceTimersByTimeAsync(0);
	expect(sent).toEqual([false, true]);
	vi.useRealTimers();
});

test('a failing send never throws', async () => {
	const r = new Reporter(async () => Promise.reject(new Error('offline')), () => ({}));
	r.urgent();
	await new Promise((res) => setTimeout(res, 0));
});
