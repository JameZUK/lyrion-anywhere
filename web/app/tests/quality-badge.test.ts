// The quality badge and the signal-path sheet on screen (docs/plans/signal-path-redesign.md):
// the badge states the weakest link, then what is heard, on Now playing (both layouts) and in
// the mini player; the sheet draws the shared model's blocks, Details, and the quality switch,
// whose override applies now with a seek. The exact case reported: a 24/96 FLAC on mobile
// data, re-encoded to Opus 160, reads "Transcoded · Opus 160" in amber.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { PlayerStatus } from '#lib/lms/types';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { device, QUALITY_ECHO_WAIT_MS } = await import('#lib/player/device.svelte');
const { SERVER_TOO_OLD_TEXT } = await import('#lib/state/status');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');
const { default: PlayerBar } = await import('#lib/components/PlayerBar.svelte');
const { default: SignalPathView } = await import('#lib/components/SignalPathView.svelte');
const { default: PlaybackSettings } = await import('#lib/settings/PlaybackSettings.svelte');
const { default: AdvancedSettings } = await import('#lib/settings/AdvancedSettings.svelte');
const { default: StatusSlot } = await import('#lib/components/StatusSlot.svelte');

const HERE = '02:4c:41:00:00:01';
const KITCHEN = '02:00:00:00:00:02';

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
	takeRecords() {
		return [];
	}
}
let width = 360;
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
	vi.stubGlobal('matchMedia', (q: string) => {
		const min = q.match(/min-width:\s*(\d+)px/);
		return { matches: min ? width >= Number(min[1]) : false, addEventListener() {}, removeEventListener() {} };
	});
	HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext;
});
afterAll(() => vi.unstubAllGlobals());

function status(over: Partial<PlayerStatus> = {}): PlayerStatus {
	return {
		mode: 'play',
		time: 30,
		duration: 240,
		can_seek: 1,
		playlist_cur_index: 0,
		playlist_tracks: 1,
		'mixer volume': 50,
		playlist_loop: [{ id: 11, title: 'Coastal song', artist: 'The Example Quartet', album: 'Coastal Hours', type: 'flc', samplesize: '24', samplerate: '96000', bitrate: '3021kbps', 'playlist index': 0 }],
		...over,
	} as PlayerStatus;
}

function network(type: string | null) {
	Object.defineProperty(navigator, 'connection', { value: type ? Object.assign(new EventTarget(), { type, saveData: false }) : undefined, configurable: true });
}

/** This device plays the reported track: the worker decodes Opus, the gateway sent Opus 160 for High. */
function reencodedHere() {
	device.applyStatus({
		output: 'Playing',
		sample_rate: 48000,
		elapsed_ms: 30_000,
		source: { codec: 'Opus', lossless: false, bits: null, channels: 2 },
		served: { codec: 'flac', rate: 96000, bits: 24, channels: 2 },
		sent: { mode: 'opus', kbps: 160, level: 'high' },
		stream_seq: 3,
	});
}

beforeEach(() => {
	fake.calls.length = 0;
	fake.handler.rpc = async () => ({});
	fake.app.players = [
		{ playerid: HERE, name: 'This browser' },
		{ playerid: KITCHEN, name: 'Kitchen' },
	];
	fake.app.deviceMac = HERE;
	fake.app.playerId = HERE;
	fake.app.now = status();
	fake.app.elapsed = 30;
	device.mac = HERE;
	device.state = 'connected';
	device.qualityOverride = null;
	device.outdated = false;
	device.serverTooOld = false;
	network('cellular');
	device.setQualityPreference('auto');
	reencodedHere();
	width = 360;
});
afterEach(() => {
	network(null);
	device.setQualityPreference('auto');
});

const blocks = (root: ParentNode) => [...root.querySelectorAll('[data-block]')].map((b) => b.getAttribute('data-block'));
const time = () => fake.calls.filter((c) => c.cmd[0] === 'time');

/**
 * A tap on the switch, then the wait for the gateway's echo of the level: no worker runs here,
 * so none comes, and the stream restarts after the wait (device-protocol2.test.ts has the echo).
 */
async function chooseAndWait(el: Element) {
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
	try {
		await fireEvent.click(el);
		vi.advanceTimersByTime(QUALITY_ECHO_WAIT_MS);
	} finally {
		vi.useRealTimers();
	}
}

describe('the badge', () => {
	test.each([
		['phones', 360],
		['wide windows', 1300],
	])('Now playing (%s): "Transcoded · Opus 160", amber', async (_name, w) => {
		width = w;
		const r = render(NowPlaying);
		await tick();
		const badge = r.container.querySelector<HTMLButtonElement>('[data-quality-badge]')!;
		expect(badge.textContent).toBe('Transcoded · Opus 160');
		expect(badge.dataset.verdict).toBe('transcoded');
		expect(badge.classList.contains('amber')).toBe(true);
		expect(badge.getAttribute('aria-label')).toBe('Transcoded · Opus 160. Show the signal path');
		r.unmount();
	});

	test('the mini player says it too, in its second line', async () => {
		const r = render(PlayerBar);
		await tick();
		const q = r.container.querySelector<HTMLElement>('[data-mini-player] [data-mini-quality]')!;
		expect(q.textContent).toBe('Transcoded · Opus 160');
		expect(q.classList.contains('amber')).toBe(true);
		expect(r.container.querySelector('.sub')!.textContent).toContain('The Example Quartet · Coastal Hours · Transcoded · Opus 160');
		r.unmount();
	});

	test('passed through as stored: Hi-Res, not amber', async () => {
		device.applyStatus({
			output: 'Playing',
			sample_rate: 96000,
			source: { codec: 'FLAC', lossless: true, bits: 24, channels: 2 },
			served: { codec: 'flac', rate: 96000, bits: 24, channels: 2 },
			sent: { mode: 'passthrough' },
		});
		const r = render(PlayerBar);
		await tick();
		const q = r.container.querySelector<HTMLElement>('[data-mini-quality]')!;
		expect(q.textContent).toBe('Hi-Res · FLAC 24/96');
		expect(q.classList.contains('amber')).toBe(false);
		r.unmount();
	});

	test("another player: the file's verdict (the gateway never transcodes for it)", async () => {
		fake.app.playerId = KITCHEN;
		const r = render(PlayerBar);
		await tick();
		expect(r.container.querySelector('[data-mini-quality]')!.textContent).toBe('Hi-Res · FLAC 24/96');
		r.unmount();
		width = 1300;
		const np = render(NowPlaying);
		await tick();
		expect(np.container.querySelector('[data-quality-badge]')!.textContent).toBe('Hi-Res · FLAC 24/96');
		np.unmount();
	});

	test('nothing playing: no badge in the mini player', async () => {
		fake.app.now = status({ playlist_loop: [] });
		const r = render(PlayerBar);
		await tick();
		expect(r.container.querySelector('[data-mini-quality]')).toBeNull();
		r.unmount();
	});
});

describe('the sheet', () => {
	test('the blocks of the reported case: File → Lyrion gateway (amber) → This device, with Details', async () => {
		const r = render(SignalPathView);
		await tick();
		const path = r.container.querySelector('[data-signal-path]')!;
		expect(blocks(path)).toEqual(['file', 'gateway', 'device']);
		const gw = path.querySelector<HTMLElement>('[data-block=gateway]')!;
		expect(gw.classList.contains('lowers')).toBe(true);
		expect(gw.querySelector('.title')!.textContent).toBe('Lyrion gateway');
		expect(gw.querySelector('.format')!.textContent).toBe('Opus 160');
		expect(gw.querySelector('.note')!.textContent).toBe('Re-encoded to Opus 160 kb/s (High, on mobile data)');
		expect(path.querySelector('[data-block=file]')!.classList.contains('lowers')).toBe(false);
		expect(path.querySelector('[data-block=file] .format')!.textContent).toBe('FLAC 24/96');
		expect(path.querySelector('[data-block=device] .format')!.textContent).toBe('Opus 48 kHz');
		// Arrows between the cards, hidden from screen readers.
		expect([...path.querySelectorAll('.arrow')].every((a) => a.getAttribute('aria-hidden') === 'true')).toBe(true);
		expect(path.querySelectorAll('.arrow')).toHaveLength(2);
		// Details under the device block: today's rows and notes, folded away.
		const details = path.querySelector<HTMLDetailsElement>('[data-block=device] [data-details]')!;
		expect(details.open).toBe(false);
		expect(details.querySelector('summary')!.textContent).toBe('Details');
		expect(details.textContent).toContain('Track in the library');
		expect(details.textContent).toContain('Stream to this device');
		expect(details.textContent).toContain('the operating system may still mix or resample');
		r.unmount();
	});

	test('the switch: the level in force selected, with why', async () => {
		const r = render(SignalPathView);
		await tick();
		const sw = r.container.querySelector('[data-block=gateway] [data-quality-switch]')!;
		const radios = [...sw.querySelectorAll<HTMLButtonElement>('[role=radio]')];
		expect(radios.map((b) => b.textContent)).toEqual(['Original', 'High', 'Data saver']);
		expect(radios.map((b) => b.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false']);
		expect(r.container.querySelector('[data-origin]')!.textContent).toBe('High, on mobile data');
		expect(r.container.querySelector('[data-back-to-automatic]')).toBeNull();
		expect(r.container.querySelector('[data-applies-next]')).toBeNull();
		r.unmount();
	});

	test('choosing Original: applies now (sought to where it is); then back to automatic', async () => {
		const r = render(SignalPathView);
		await tick();
		await chooseAndWait(r.container.querySelector('[data-level=original]')!);
		expect(device.qualityOverride).toBe('original');
		expect(time()).toEqual([{ player: HERE, cmd: ['time', 30] }]);
		await waitFor(() => expect(r.container.querySelector('[data-origin]')!.textContent).toBe('Original, overridden here'));
		expect(r.container.querySelector('[data-level=original]')!.getAttribute('aria-checked')).toBe('true');
		// The gateway still says Opus until the new stream's message arrives; then it passes through.
		device.applyStatus({
			output: 'Playing',
			sample_rate: 96000,
			source: { codec: 'FLAC', lossless: true, bits: 24, channels: 2 },
			served: { codec: 'flac', rate: 96000, bits: 24, channels: 2 },
			sent: { mode: 'passthrough' },
		});
		await tick();
		expect(r.container.querySelector('.verdict')!.textContent).toBe('Hi-Res · FLAC 24/96');
		await chooseAndWait(r.container.querySelector('[data-back-to-automatic]')!);
		expect(device.qualityOverride).toBeNull();
		await waitFor(() => expect(r.container.querySelector('[data-origin]')!.textContent).toBe('High, on mobile data'));
		expect(time()).toHaveLength(2);
		r.unmount();
	});

	test('a stream that cannot be sought: the switch says it applies from the next track, and seeks nothing', async () => {
		fake.app.now = status({ duration: 0, can_seek: 0 });
		const r = render(SignalPathView);
		await tick();
		expect(r.container.querySelector('[data-applies-next]')!.textContent).toBe('Applies from the next track.');
		await fireEvent.click(r.container.querySelector('[data-level=low]')!);
		expect(device.qualityOverride).toBe('low');
		expect(time()).toEqual([]);
		r.unmount();
	});

	test('Safari (no Network Information API): "chosen in Settings", and the switch works', async () => {
		network(null);
		device.setQualityPreference('high');
		const r = render(SignalPathView);
		await tick();
		expect(r.container.querySelector('[data-origin]')!.textContent).toBe('High, chosen in Settings');
		await chooseAndWait(r.container.querySelector('[data-level=original]')!);
		expect(time()).toHaveLength(1);
		await waitFor(() => expect(r.container.querySelector('[data-origin]')!.textContent).toBe('Original, overridden here'));
		r.unmount();
	});

	test('another player: File → that player, no gateway, no switch', async () => {
		fake.app.playerId = KITCHEN;
		const r = render(SignalPathView);
		await tick();
		const path = r.container.querySelector('[data-signal-path]')!;
		expect(blocks(path)).toEqual(['file', 'player']);
		expect(path.querySelector('[data-block=player] .title')!.textContent).toBe('Kitchen');
		expect(path.querySelector('[data-block=player] .note')!.textContent).toBe('Plays the stream from LMS');
		expect(path.querySelector('[data-quality-switch]')).toBeNull();
		// Details are still there, under the player.
		expect(path.querySelector('[data-block=player] [data-details]')!.textContent).toContain('On Kitchen');
		r.unmount();
	});

	test('a lossy file on another player: the file block is amber', async () => {
		fake.app.playerId = KITCHEN;
		fake.app.now = status({ playlist_loop: [{ id: 12, title: 'Song', type: 'mp3', bitrate: '320kbps' }] as PlayerStatus['playlist_loop'] });
		const r = render(SignalPathView);
		await tick();
		expect(r.container.querySelector('.verdict')!.textContent).toBe('Lossy · MP3 320');
		expect(r.container.querySelector('[data-block=file]')!.classList.contains('lowers')).toBe(true);
		r.unmount();
	});
});

describe('around the app', () => {
	test("Settings' streaming quality says when an override is in force", async () => {
		const r = render(PlaybackSettings);
		await tick();
		expect(r.container.textContent).not.toContain('overridden on the signal path');
		device.overrideQuality('original', { seekable: false });
		await tick();
		expect(r.container.textContent).toContain('Now: Original, overridden on the signal path. This setting applies again when the page reloads or the network changes.');
		r.unmount();
	});

	test('a server too old for this browser: Advanced says so, and the player cannot be started', async () => {
		device.state = 'off';
		device.serverTooOld = true;
		const r = render(AdvancedSettings);
		await tick();
		expect(r.container.querySelector('[data-server-too-old]')!.textContent).toBe(SERVER_TOO_OLD_TEXT);
		expect(SERVER_TOO_OLD_TEXT).toBe("This browser can't play from this server until its Lyrion Anywhere plugin is updated to 0.7.3 or later. You can still control other players.");
		expect(r.container.querySelector<HTMLButtonElement>('[data-player-start]')!.disabled).toBe(true);
		r.unmount();
	});

	test('a stale page (4426): the status line says to reload, with a Reload button', async () => {
		device.state = 'stopped';
		device.outdated = true;
		const r = render(StatusSlot);
		await tick();
		const slot = r.container.querySelector('[data-status=outdated]')!;
		expect(slot.textContent).toContain('This page is out of date: reload to update the app');
		expect(slot.querySelector('[data-outdated-reload]')).not.toBeNull();
		r.unmount();
	});
});
