// The one status slot (Wave 3.5, rule 1): lasting states share one line, in priority order
// (moving > out of reach > busy > reconnecting, then this device's two offers), and only one
// shows. Results go to the one snackbar (app.notice and app.error included).
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { statusOf, HELD_TEXT, RECONNECTING_TEXT, RESUME_TEXT, type StatusInput } from '$lib/state/status';
import { BUSY_TEXT, CONNECTION_ID, UNREACHABLE_TEXT } from '$lib/state/reachability';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const fake = await import('./helpers/fake-app.svelte');
const { device } = await import('$lib/player/device.svelte');
const { default: StatusSlot } = await import('$lib/components/StatusSlot.svelte');

const NONE: StatusInput = { moving: '', reach: 'online', message: '', reconnecting: false, heldBack: false, resume: false };

describe('statusOf: one line, the most important first', () => {
	test('nothing to say: nothing shown', () => {
		expect(statusOf(NONE)).toBeNull();
	});

	test('the order is moving > out of reach > busy > reconnecting > sound held back > resume', () => {
		const all: StatusInput = { moving: 'Kitchen', reach: 'unreachable', message: UNREACHABLE_TEXT, reconnecting: true, heldBack: true, resume: true };
		expect(statusOf(all)).toEqual({ kind: 'moving', text: 'Moving playback to Kitchen…' });
		expect(statusOf({ ...all, moving: '' })).toEqual({ kind: 'unreachable', text: UNREACHABLE_TEXT });
		expect(statusOf({ ...all, moving: '', reach: 'busy', message: BUSY_TEXT })).toEqual({ kind: 'busy', text: BUSY_TEXT });
		expect(statusOf({ ...all, moving: '', reach: 'online', message: '' })).toEqual({ kind: 'reconnecting', text: RECONNECTING_TEXT });
		expect(statusOf({ ...NONE, heldBack: true, resume: true })).toEqual({ kind: 'held', text: HELD_TEXT });
		expect(statusOf({ ...NONE, resume: true })).toEqual({ kind: 'resume', text: RESUME_TEXT });
	});

	test('a reach state with no message says nothing (the message decides the words)', () => {
		expect(statusOf({ ...NONE, reach: 'unreachable' })).toBeNull();
	});

	test('a hostile player name stays text', () => {
		expect(statusOf({ ...NONE, moving: '<img src=x onerror=alert(1)>' })!.text).toBe('Moving playback to <img src=x onerror=alert(1)>…');
	});
});

describe('the slot', () => {
	beforeEach(() => {
		fake.app.moving = '';
		fake.app.reach = 'online';
		fake.app.playerId = '';
		fake.app.players = [];
		device.state = 'off';
		device.wasActive = false;
		device.mac = '';
	});

	test('out of reach: one line with the connection message, carrying the id disabled controls point at', async () => {
		fake.app.reach = 'unreachable';
		const r = render(StatusSlot);
		await tick();
		const slots = r.container.querySelectorAll('[data-status]');
		expect(slots).toHaveLength(1);
		expect(slots[0]!.id).toBe(CONNECTION_ID);
		expect(slots[0]!.getAttribute('role')).toBe('status');
		expect(slots[0]!.textContent).toContain('Can’t reach your music server.');
	});

	test('moving outranks out of reach: still one line', async () => {
		fake.app.reach = 'unreachable';
		fake.app.moving = 'Kitchen';
		const r = render(StatusSlot);
		await tick();
		expect(r.container.querySelectorAll('[data-status]')).toHaveLength(1);
		expect(r.container.querySelector('[data-status]')!.getAttribute('data-status')).toBe('moving');
	});

	test('after a reload: the offer to resume here, with Resume and Dismiss; no placebo button for a held sound', async () => {
		device.wasActive = true;
		device.state = 'stopped';
		const start = vi.spyOn(device, 'start').mockResolvedValue(undefined);
		const r = render(StatusSlot);
		await tick();
		const slot = r.container.querySelector('[data-status="resume"]')!;
		expect(slot.textContent).toContain(RESUME_TEXT);
		await fireEvent.click(r.getByRole('button', { name: 'Resume playing here' }));
		expect(start).toHaveBeenCalledTimes(1);
		start.mockRestore();
	});
});
