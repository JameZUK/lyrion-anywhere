// Components the October review found untested: a rendering and a main-interaction test for
// each (PlaylistsView, AppearancePicker, AdvancedSettings), and the PickerRow radiogroup's
// arrow keys. AccountSettings, SaveQueueDialog, DownloadBar and ArtistDownload are covered in
// settings-page, save-queue and download-remove.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('$app/navigation', () => ({ goto: vi.fn(async () => {}) }));
const fake = await import('./helpers/fake-app.svelte');
const { default: PlaylistsView } = await import('$lib/components/PlaylistsView.svelte');
const { default: AppearancePicker } = await import('$lib/components/AppearancePicker.svelte');
const { appearance } = await import('$lib/theme/appearance.svelte');
const { THEMES } = await import('$lib/theme/themes');
const { default: AdvancedSettings } = await import('$lib/settings/AdvancedSettings.svelte');
const { default: PickerRow } = await import('$lib/components/PickerRow.svelte');

const settle = async () => {
	for (let i = 0; i < 5; i++) {
		await tick();
		await new Promise((r) => setTimeout(r, 0));
	}
};

describe('PlaylistsView', () => {
	let lists: { id: number; playlist: string }[] = [];
	let createGate: Promise<void> | null = null;

	beforeEach(() => {
		fake.calls.length = 0;
		fake.app.isAdmin = true;
		lists = [
			{ id: 1, playlist: 'Road Trip' },
			{ id: 2, playlist: '<img src=x onerror=alert(1)>' },
		];
		fake.handler.rpc = async (_p, c) => {
			if (c[0] === 'playlists' && c[1] === 'new') {
				if (createGate) await createGate;
				const id = 10 + lists.length;
				lists = [...lists, { id, playlist: String(c[2]).replace(/^name:/, '') }];
				return { playlist_id: id };
			}
			if (c[0] === 'playlists') return { playlists_loop: lists, count: lists.length };
			return {};
		};
	});
	afterEach(() => {
		createGate = null;
		fake.app.isAdmin = false;
		fake.handler.rpc = async () => ({});
	});

	test('lists the saved playlists, names as text', async () => {
		const r = render(PlaylistsView);
		await settle();
		const names = [...r.container.querySelectorAll('.name')].map((n) => n.textContent);
		expect(names).toEqual(['Road Trip', '<img src=x onerror=alert(1)>']);
		expect(r.container.querySelector('img')).toBeNull();
		expect(r.container.querySelector<HTMLAnchorElement>('a.main')!.getAttribute('href')).toContain('playlist/1');
	});

	test('listeners get no New playlist', async () => {
		fake.app.isAdmin = false;
		render(PlaylistsView);
		await settle();
		expect(screen.queryByRole('button', { name: 'New playlist' })).toBeNull();
	});

	// Regression (review, Web low): a double click on Create made two playlists.
	test('Create makes one playlist however often it is pressed', async () => {
		let release!: () => void;
		createGate = new Promise((r) => (release = r));
		const r = render(PlaylistsView);
		await settle();
		await fireEvent.click(screen.getByRole('button', { name: 'New playlist' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'Late night' } });
		const form = r.container.querySelector('form.create')!;
		await fireEvent.submit(form);
		await fireEvent.submit(form);
		expect((screen.getByRole('button', { name: /Create|Creating/ }) as HTMLButtonElement).disabled).toBe(true);
		release();
		await settle();
		expect(fake.calls.filter((c) => c.cmd[0] === 'playlists' && c.cmd[1] === 'new')).toHaveLength(1);
		const { goto } = await import('$app/navigation');
		await waitFor(() => expect(goto).toHaveBeenCalled());
	});

	test('a failed create says why and lets you try again', async () => {
		fake.handler.rpc = async (_p, c) => {
			if (c[1] === 'new') throw new Error('Couldn’t reach your music server.');
			return { playlists_loop: [], count: 0 };
		};
		const r = render(PlaylistsView);
		await settle();
		await fireEvent.click(screen.getByRole('button', { name: 'New playlist' }));
		await fireEvent.input(screen.getByLabelText('Name'), { target: { value: 'X' } });
		await fireEvent.submit(r.container.querySelector('form.create')!);
		await settle();
		expect(r.container.querySelector('.error')!.textContent).toContain('Couldn’t reach');
		expect((screen.getByRole('button', { name: 'Create' }) as HTMLButtonElement).disabled).toBe(false);
	});
});

describe('AppearancePicker', () => {

	test('shows every theme and the light/dark choice; choosing one applies it', async () => {
		const before = { ...appearance.prefs };
		try {
			const r = render(AppearancePicker);
			const themes = r.container.querySelectorAll('[aria-label="Theme"] input[type=radio]');
			expect(themes).toHaveLength(THEMES.length);
			const other = THEMES.find((t) => t.id !== appearance.prefs.theme)!;
			const label = [...r.container.querySelectorAll('.theme')].find((l) => l.querySelector('.name')!.textContent === other.name)!;
			await fireEvent.click(label.querySelector('input')!);
			expect(appearance.prefs.theme).toBe(other.id);
			await fireEvent.click(screen.getByLabelText('Dark'));
			expect(appearance.prefs.mode).toBe('dark');
			await tick();
			expect(screen.getByLabelText<HTMLInputElement>('Dark').checked).toBe(true);
		} finally {
			appearance.set(before);
		}
	});
});

describe('AdvancedSettings', () => {

	test('shows this device’s player and the engine; picking an engine saves it', async () => {
		localStorage.removeItem('anywhere.playback.mode');
		const r = render(AdvancedSettings);
		expect(r.container.querySelector('[data-player-state]')!.textContent).toContain('Not running');
		const row = r.container.querySelector<HTMLButtonElement>('[data-setting="playback-engine"]')!;
		expect(row.querySelector('[data-setting-value]')!.textContent).toBe('Automatic');
		await fireEvent.click(row);
		const picker = await waitFor(() => document.querySelector<HTMLElement>('[data-picker="playback-engine"]')!);
		await fireEvent.click(picker.querySelector('[data-choice="media"]')!);
		await tick();
		expect(localStorage.getItem('anywhere.playback.mode')).toBe('media');
		expect(row.querySelector('[data-setting-value]')!.textContent).toBe('Media element');
		localStorage.removeItem('anywhere.playback.mode');
		r.unmount();
	});
});

describe('PickerRow: a radiogroup the keyboard can drive', () => {
	const options = [
		{ value: 'a', label: 'First' },
		{ value: 'b', label: 'Second', disabled: true },
		{ value: 'c', label: 'Third' },
	];

	async function open(value = 'a') {
		const onchange = vi.fn();
		const r = render(PickerRow, { label: 'Pick', value, options, onchange, id: 'pick' });
		await fireEvent.click(r.container.querySelector('[data-setting="pick"]')!);
		const group = await waitFor(() => document.querySelector<HTMLElement>('[data-picker="pick"]')!);
		const radio = (v: string) => group.querySelector<HTMLButtonElement>(`[data-choice="${v}"]`)!;
		return { r, group, radio, onchange };
	}

	test('only the chosen one is in the tab order; it has the focus on opening', async () => {
		const { r, radio } = await open('c');
		expect(radio('c').tabIndex).toBe(0);
		expect(radio('a').tabIndex).toBe(-1);
		await waitFor(() => expect(document.activeElement).toBe(radio('c')));
		r.unmount();
	});

	test('arrow keys move between the choices (skipping unavailable ones, wrapping round); Home and End too', async () => {
		const { r, group, radio } = await open('a');
		radio('a').focus();
		await fireEvent.keyDown(group, { key: 'ArrowDown' });
		expect(document.activeElement).toBe(radio('c')); // b is unavailable
		await fireEvent.keyDown(group, { key: 'ArrowRight' });
		expect(document.activeElement).toBe(radio('a')); // round to the start
		await fireEvent.keyDown(group, { key: 'ArrowUp' });
		expect(document.activeElement).toBe(radio('c'));
		await fireEvent.keyDown(group, { key: 'Home' });
		expect(document.activeElement).toBe(radio('a'));
		await fireEvent.keyDown(group, { key: 'End' });
		expect(document.activeElement).toBe(radio('c'));
		r.unmount();
	});

	test('Enter or Space on the focused choice picks it', async () => {
		const { r, radio, onchange } = await open('a');
		radio('c').focus();
		await fireEvent.click(radio('c')); // a button: Enter and Space are a click
		expect(onchange).toHaveBeenCalledWith('c');
		r.unmount();
	});
});
