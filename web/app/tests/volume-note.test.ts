// When "this device" plays at full scale and its own buttons set the level (iPhone/iPad), the
// volume slider is replaced by a note, in the player bar and in Now playing (on phones, in the
// Volume popover). Other players, and this device when it follows LMS, keep the slider.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import PlayerBar from '$lib/components/PlayerBar.svelte';
import NowPlaying from '../src/routes/+page.svelte';
import { app } from '$lib/state/app.svelte';
import { device } from '$lib/player/device.svelte';
import { DEVICE_BUTTONS_NOTE, planVolume } from '$lib/player/volume';

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

/** Now playing on phones keeps the volume in a popover: open it (the player bar has none). */
async function openVolume(c: HTMLElement) {
	const b = c.querySelector('[data-volume-button]');
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
