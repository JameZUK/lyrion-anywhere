// "Output level is fixed at 100%": any player whose status says LMS keeps its output level at
// 100% (`digital_volume_control: 0`), this device or another (a Squeezebox set that way), shows
// the note where its volume slider would be, and the volume keys leave it alone. Settings ›
// About › Advanced has the switch for this device's own player, on by default.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { PlayerStatus } from '#lib/lms/types';
import type { KeyLike } from '#lib/keys/shortcuts';

vi.mock('#lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const url = new URL('http://app.example/#/');
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return url;
		},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { device } = await import('#lib/player/device.svelte');
const { DEVICE_BUTTONS_NOTE, FIXED_LEVEL_LABEL, FIXED_LEVEL_NOTE, FIXED_LEVEL_NO_FOLLOW, planVolume } = await import('#lib/player/volume');
const { default: PlayerBar } = await import('#lib/components/PlayerBar.svelte');
const { default: VolumeSlider } = await import('#lib/components/VolumeSlider.svelte');
const { default: RoomCard } = await import('#lib/components/RoomCard.svelte');
const { default: KeyboardShortcuts } = await import('#lib/components/KeyboardShortcuts.svelte');
const { default: AdvancedSettings } = await import('#lib/settings/AdvancedSettings.svelte');
const { default: NowPlaying } = await import('../src/routes/+page.svelte');
const { nowPlaying } = await import('#lib/nowplaying/store.svelte');

const HERE = '02:00:00:00:00:0a';
const KITCHEN = '02:00:00:00:00:0b';

class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
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
	return { mode: 'play', time: 30, duration: 240, power: 1, 'mixer volume': 40, playlist_loop: [{ id: 11, title: 'Harbour Lights' }], ...over };
}

beforeEach(() => {
	fake.calls.length = 0;
	fake.app.players = [
		{ playerid: HERE, name: 'This browser' },
		{ playerid: KITCHEN, name: 'Kitchen' },
	];
	fake.app.deviceMac = HERE;
	fake.app.playerId = KITCHEN;
	fake.app.now = status();
	device.mac = HERE;
	device.volumePlan = null;
	width = 360;
	url.hash = '#/';
	fake.handler.rpc = async () => ({});
});
afterEach(() => {
	device.mac = '';
	device.volumePlan = null;
	localStorage.clear();
	device.fixedLevel = true;
});

const slider = (c: Element) => c.querySelector('input[type=range][max="100"]');
const fixedNote = (c: Element) => c.querySelector('[data-fixed-level-note]');

/** Now playing: on phones the volume is in Play on… (open it); wide, it is on the page. */
async function nowPlayingAt(w: number) {
	width = w;
	const r = render(NowPlaying);
	await tick();
	if (w < 840) {
		await fireEvent.click(r.container.querySelector('[data-play-on]')!);
		return { r, where: r.container.querySelector<HTMLElement>('[role=dialog][aria-label="Play on"]')! };
	}
	return { r, where: r.container };
}

describe('the volume sliders', () => {
	for (const [name, View] of [
		['player bar', PlayerBar],
		['the Play on volume slider', VolumeSlider],
	] as const) {
		test(`${name}: a player at a fixed output level shows the note, not the slider`, async () => {
			fake.app.now = status({ digital_volume_control: 0 });
			const r = render(View);
			await tick();
			expect(fixedNote(r.container)!.textContent).toContain(FIXED_LEVEL_NOTE);
			expect(slider(r.container)).toBeNull();
			r.unmount();
		});

		test(`${name}: Volume Control on (or not reported) keeps the slider`, async () => {
			for (const s of [status({ digital_volume_control: 1 }), status()]) {
				fake.app.now = s;
				const r = render(View);
				await tick();
				expect(fixedNote(r.container)).toBeNull();
				expect(slider(r.container)).not.toBeNull();
				r.unmount();
			}
		});

		test(`${name}: the fixed-level note wins over the device-buttons note`, async () => {
			fake.app.playerId = HERE;
			device.volumePlan = planVolume({ mode: 'media', mediaVolume: false }, 'auto');
			fake.app.now = status({ digital_volume_control: 0 });
			let r = render(View);
			await tick();
			expect(r.container.textContent).toContain(FIXED_LEVEL_NOTE);
			expect(r.container.textContent).not.toContain(DEVICE_BUTTONS_NOTE);
			r.unmount();
			// Not fixed: the device-buttons note, as before.
			fake.app.now = status({ digital_volume_control: 1 });
			r = render(View);
			await tick();
			expect(r.container.textContent).toContain(DEVICE_BUTTONS_NOTE);
			expect(fixedNote(r.container)).toBeNull();
			r.unmount();
		});
	}

	for (const w of [360, 1200]) {
		test(`Now playing (${w < 840 ? 'phone' : 'wide'}): the note for a fixed level, the slider otherwise`, async () => {
			fake.app.now = status({ digital_volume_control: 0 });
			let { r, where } = await nowPlayingAt(w);
			expect(fixedNote(where)!.textContent).toContain(FIXED_LEVEL_NOTE);
			expect(slider(where)).toBeNull();
			r.unmount();
			fake.app.now = status({ digital_volume_control: 1 });
			({ r, where } = await nowPlayingAt(w));
			expect(fixedNote(where)).toBeNull();
			expect(slider(where)).not.toBeNull();
			r.unmount();
		});
	}

	test('Rooms: a room at a fixed output level shows the note in place of its slider', async () => {
		const props = { id: KITCHEN, name: 'Kitchen', actions: [], oncontrol() {}, onplaypause() {}, onvolume() {}, onpower() {}, ondrag() {} };
		let r = render(RoomCard, { ...props, status: status({ digital_volume_control: 0 }) });
		expect(fixedNote(r.container)!.textContent).toBe(FIXED_LEVEL_NOTE);
		expect(r.container.querySelector('[data-room-volume]')).toBeNull();
		r.unmount();
		r = render(RoomCard, { ...props, status: status({ digital_volume_control: 1 }) });
		expect(fixedNote(r.container)).toBeNull();
		expect(r.container.querySelector('[data-room-volume]')).not.toBeNull();
		r.unmount();
	});
});

describe('the volume keys', () => {
	const press = (k: string) => fireEvent.keyDown(document.body, { key: k } as KeyLike);
	const volumes = () => fake.calls.filter((c) => c.cmd[0] === 'mixer');

	test('do nothing for a player at a fixed output level, and work otherwise', async () => {
		nowPlaying.setLayout('art');
		const r = render(KeyboardShortcuts);
		fake.app.now = status({ digital_volume_control: 0 });
		await press('ArrowUp');
		await press('ArrowDown');
		expect(volumes()).toEqual([]);
		fake.app.now = status({ digital_volume_control: 1 });
		await press('ArrowUp');
		expect(volumes().map((c) => c.cmd)).toEqual([['mixer', 'volume', 45]]);
		r.unmount();
	});
});

describe('Settings › About › Advanced', () => {
	const sw = (c: Element) => c.querySelector<HTMLButtonElement>('[data-fixed-level]')!;
	const picker = (c: Element) => c.querySelector<HTMLButtonElement>('[data-setting="volume-source"]')!;

	test('the switch is on by default, above the volume picker, which it disables', async () => {
		localStorage.clear();
		device.fixedLevel = true;
		const r = render(AdvancedSettings);
		const s = sw(r.container);
		expect(s.getAttribute('role')).toBe('switch');
		expect(s.getAttribute('aria-checked')).toBe('true');
		expect(s.getAttribute('aria-label')).toBe(FIXED_LEVEL_LABEL);
		expect(s.textContent).toContain('On');
		expect(s.textContent).toContain('no controller can turn it down');
		expect(s.compareDocumentPosition(picker(r.container)) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
		expect(picker(r.container).disabled).toBe(true);
		expect(picker(r.container).textContent).toContain(FIXED_LEVEL_NO_FOLLOW);
		r.unmount();
	});

	test('turning it off saves it and lets the volume be chosen again; on again forgets it', async () => {
		const r = render(AdvancedSettings);
		await fireEvent.click(sw(r.container));
		await tick();
		expect(device.fixedLevel).toBe(false);
		expect(sw(r.container).getAttribute('aria-checked')).toBe('false');
		expect(sw(r.container).textContent).toContain('Off');
		expect(localStorage.getItem('anywhere.device.fixedLevel')).toBe('off');
		expect(picker(r.container).disabled).toBe(false);
		expect(picker(r.container).textContent).not.toContain(FIXED_LEVEL_NO_FOLLOW);
		await fireEvent.click(sw(r.container));
		await waitFor(() => expect(picker(r.container).disabled).toBe(true));
		expect(localStorage.getItem('anywhere.device.fixedLevel')).toBeNull();
		r.unmount();
	});
});
