// When "this device" plays at full scale and its own buttons set the level (iPhone/iPad), the
// volume slider is replaced by a note, in the player bar and in Now playing (on phones, in
// Play on…). Other players, and this device when it follows LMS, keep the slider.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import PlayerBar from '#lib/components/PlayerBar.svelte';
import NowPlaying from '../src/routes/+page.svelte';
import { app } from '#lib/state/app.svelte';
import { device } from '#lib/player/device.svelte';
import { DEVICE_BUTTONS_NOTE, planVolume } from '#lib/player/volume';

// jsdom has no layout observers (used by artwork and the seek bar).
class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
}
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
});
afterAll(() => vi.unstubAllGlobals());

const MAC = '02:00:00:00:00:01';
const OTHER = '02:00:00:00:00:02';

function setUp(selected: string, plan: ReturnType<typeof planVolume> | null) {
	app.playerId = selected;
	app.now = { mode: 'play', 'mixer volume': 40, playlist_loop: [] } as unknown as typeof app.now;
	device.mac = MAC;
	device.volumePlan = plan;
}

/** Now playing on phones keeps the volume in Play on… (Wave 3.5): open it (the player bar has its own). */
async function openVolume(c: HTMLElement) {
	const b = c.querySelector('[data-play-on]');
	if (b) await fireEvent.click(b);
}

afterEach(() => {
	device.mac = '';
	device.volumePlan = null;
	app.playerId = '';
	app.now = null;
});

for (const [name, View] of [
	['player bar', PlayerBar],
	['Now playing', NowPlaying],
] as const) {
	test(`${name}: this device on its own buttons shows the note instead of the slider`, async () => {
		setUp(MAC, planVolume({ mode: 'media', mediaVolume: false }, 'auto'));
		const r = render(View);
		await tick();
		await openVolume(r.container);
		expect(r.getByText(DEVICE_BUTTONS_NOTE)).toBeTruthy();
		expect(r.container.querySelector('input[type=range][max="100"]')).toBeNull();
		r.unmount();
	});

	test(`${name}: following LMS, or another player, keeps the slider`, async () => {
		setUp(MAC, planVolume({ mode: 'media', mediaVolume: true }, 'auto'));
		let r = render(View);
		await tick();
		await openVolume(r.container);
		expect(r.queryByText(DEVICE_BUTTONS_NOTE)).toBeNull();
		expect(r.container.querySelector('input[type=range][max="100"]')).not.toBeNull();
		r.unmount();

		setUp(OTHER, planVolume({ mode: 'media', mediaVolume: false }, 'auto'));
		r = render(View);
		await tick();
		await openVolume(r.container);
		expect(r.queryByText(DEVICE_BUTTONS_NOTE)).toBeNull();
		r.unmount();
	});
}

// Review (Web nit): a volume change still waiting to be sent went to the newly chosen player.
test('player bar: switching players drops a volume change not yet sent', async () => {
	setUp(OTHER, null);
	const send = vi.spyOn(app, 'send').mockResolvedValue(undefined);
	try {
		const r = render(PlayerBar);
		await tick();
		const slider = r.container.querySelector<HTMLInputElement>('input[type=range][max="100"]')!;
		await fireEvent.input(slider, { target: { value: '70' } });
		app.playerId = '02:00:00:00:00:03';
		await tick();
		await new Promise((res) => setTimeout(res, 250));
		expect(send.mock.calls.filter(([c]) => c[1] === 'volume')).toEqual([]);
		// On the same player, the change goes as usual.
		const again = r.container.querySelector<HTMLInputElement>('input[type=range][max="100"]')!;
		await fireEvent.input(again, { target: { value: '30' } });
		await new Promise((res) => setTimeout(res, 250));
		expect(send.mock.calls.filter(([c]) => c[1] === 'volume').map(([c]) => c[2])).toEqual([30]);
		r.unmount();
	} finally {
		send.mockRestore();
	}
});
