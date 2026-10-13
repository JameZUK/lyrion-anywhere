// While this device's player gets going, Now playing says what is happening and how long it
// has taken, and after a few seconds offers a way not to wait.
import { fireEvent, render } from '@testing-library/svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const device = { state: 'starting', stop: vi.fn(), start: vi.fn(async () => {}) };
vi.mock('$lib/player/device.svelte', () => ({ device }));
const fake = await import('./helpers/fake-app.svelte');
const { default: DeviceWait } = await import('$lib/components/DeviceWait.svelte');

beforeEach(() => {
	vi.useFakeTimers();
	fake.app.deviceFailures = 0;
	device.state = 'starting';
	device.stop.mockClear();
	device.start.mockClear();
});
afterEach(() => vi.useRealTimers());

test('says what is happening, with progress', async () => {
	const r = render(DeviceWait, { limitMs: 20_000, offerAfterMs: 5_000 });
	expect(r.getByRole('status').textContent).toBe('Starting this device’s player…');
	const bar = r.getByRole('progressbar', { name: 'Waiting for this device' });
	expect(bar.getAttribute('aria-valuemax')).toBe('20');
	await vi.advanceTimersByTimeAsync(3_000);
	expect(bar.getAttribute('aria-valuenow')).toBe('3');
	expect(r.queryByRole('button', { name: 'Use another player' })).toBeNull();
	r.unmount();
});

test('after a few seconds: another player, or try again', async () => {
	const r = render(DeviceWait, { limitMs: 20_000, offerAfterMs: 5_000 });
	await vi.advanceTimersByTimeAsync(5_500);
	expect(r.getByRole('status').textContent).toContain('taking longer than usual');
	await fireEvent.click(r.getByRole('button', { name: 'Try again' }));
	expect(device.stop).toHaveBeenCalledTimes(1);
	expect(device.start).toHaveBeenCalledTimes(1);
	await fireEvent.click(r.getByRole('button', { name: 'Use another player' }));
	expect(fake.app.deviceFailures).toBe(1);
	r.unmount();
});
