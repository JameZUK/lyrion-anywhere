// Wave 3.5, phase 2 (docs/plans/wave3_5-declutter.md): the one switch and the one picker,
// Library as one model (every section shown in place, the library choice in ⋯), and "Smart
// playlists" as the one name. The settings index, the status slot and the sidebar have files
// of their own (settings-page, status-slot, nav-tabs).
import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const params: Record<string, string | undefined> = {};
vi.mock('$app/state', () => ({ page: { params, url: new URL('http://app.example/#/library') } }));
vi.mock('$lib/state/me', () => ({
	routine: {},
	me: { filters: async () => [], shelf: async () => [], stations: async () => [] },
}));
const fake = await import('./helpers/fake-app.svelte');
const { default: Switch } = await import('$lib/components/Switch.svelte');
const { default: PickerRow } = await import('$lib/components/PickerRow.svelte');
const { default: LibraryPage } = await import('../src/routes/library/+page.svelte');
const { LIBRARY_KEY, CHIPS, CHIP_LABELS } = await import('$lib/library/prefs');

const HOSTILE = '<img src=x onerror=alert(1)>';
class NoObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
	takeRecords() {
		return [];
	}
}
beforeAll(() => {
	vi.stubGlobal('IntersectionObserver', NoObserver);
	vi.stubGlobal('ResizeObserver', NoObserver);
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
	fake.calls.length = 0;
	fake.app.players = [{ playerid: '02:00:00:00:00:01', name: 'Lounge' }];
	fake.app.playerId = '02:00:00:00:00:01';
	fake.handler.rpc = async (_p, c) => {
		if (c[0] === 'genres') return { count: 1, genres_loop: [{ id: 3, genre: 'Jazz' }] };
		if (c[0] === 'years') return { count: 1, years_loop: [{ year: 1999 }] };
		if (c[0] === 'musicfolder') return { count: 1, folder_loop: [{ id: 21, filename: 'Albums', type: 'folder' }] };
		if (c[0] === 'albums') return { count: 1, albums_loop: [{ id: 5, album: 'Five' }] };
		return {};
	};
});

describe('the one switch', () => {
	test('a row-wide switch: its label is its name, the state in words beside it, a click flips it', async () => {
		const seen: boolean[] = [];
		const r = render(Switch, { checked: false, label: 'Haptics', note: 'One sentence.', onchange: (v: boolean) => seen.push(v) });
		const sw = r.getByRole('switch', { name: 'Haptics' });
		expect(sw.getAttribute('aria-checked')).toBe('false');
		expect(sw.textContent).toContain('Off');
		expect(document.getElementById(sw.getAttribute('aria-describedby')!)!.textContent).toBe('One sentence.');
		await fireEvent.click(sw);
		expect(seen).toEqual([true]);
		await r.rerender({ checked: true, label: 'Haptics', onchange: () => {} });
		expect(sw.textContent).toContain('On');
	});

	test('disabled: nothing happens; a hostile label stays text', async () => {
		const seen: boolean[] = [];
		const r = render(Switch, { checked: false, label: HOSTILE, disabled: true, onchange: (v: boolean) => seen.push(v) });
		const sw = r.getByRole('switch') as HTMLButtonElement;
		expect(sw.disabled).toBe(true);
		sw.click(); // as a browser does: nothing, for a disabled button
		await tick();
		expect(seen).toEqual([]);
		expect(r.container.querySelector('img')).toBeNull();
		expect(sw.textContent).toContain(HOSTILE);
	});
});

describe('the one picker', () => {
	const OPTIONS = [
		{ value: 'a', label: 'First' },
		{ value: 'b', label: 'Second', note: 'With a note' },
		{ value: 'c', label: HOSTILE, disabled: true },
	];

	test('a row with its value; a tap opens a radio group in a sheet; choosing sets it and closes', async () => {
		const seen: string[] = [];
		const r = render(PickerRow, { label: 'Pick one', value: 'a', options: OPTIONS, onchange: (v: string) => seen.push(v), id: 'one' });
		const row = r.container.querySelector<HTMLButtonElement>('[data-setting="one"]')!;
		expect(row.querySelector('[data-setting-value]')!.textContent).toBe('First');
		await fireEvent.click(row);
		const group = await waitFor(() => document.querySelector<HTMLElement>('[data-picker="one"]')!);
		const radios = within(group).getAllByRole('radio');
		expect(radios.map((x) => x.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false']);
		expect((radios[2] as HTMLButtonElement).disabled).toBe(true);
		expect(group.querySelector('img')).toBeNull();
		await fireEvent.click(radios[1]!);
		expect(seen).toEqual(['b']);
		await waitFor(() => expect(document.querySelector('[data-picker="one"]')).toBeNull());
	});

	test('choosing the value already chosen sends nothing', async () => {
		const seen: string[] = [];
		const r = render(PickerRow, { label: 'Pick one', value: 'a', options: OPTIONS, onchange: (v: string) => seen.push(v), id: 'two' });
		await fireEvent.click(r.container.querySelector('[data-setting="two"]')!);
		await fireEvent.click(await waitFor(() => document.querySelector<HTMLElement>('[data-picker="two"] [data-choice="a"]')!));
		expect(seen).toEqual([]);
	});
});

describe('Library: one model', () => {
	async function open(chip: string) {
		localStorage.setItem(LIBRARY_KEY, JSON.stringify({ chip, sort: 'album', view: 'grid' }));
		const r = render(LibraryPage);
		await tick();
		return r;
	}

	test('every section is one chip of one row; above the first album there is only the heading, that row and the album bar', async () => {
		const r = await open('albums');
		expect([...r.container.querySelectorAll('[role=tab]')].map((t) => t.textContent?.trim())).toEqual(CHIPS.map((c) => CHIP_LABELS[c]));
		expect(r.container.querySelector('[role=tablist]')!.classList.contains('scroll')).toBe(true);
		// No second row of link pills, and no library select of its own.
		expect(r.container.querySelector('nav.browse, .picker, select[aria-label="Library"]')).toBeNull();
		r.unmount();
	});

	test('Genres, Years, Folders, Apps & radio and Downloads show in place, not on another page', async () => {
		for (const [chip, view, text] of [
			['genres', 'GenresView', 'Jazz'],
			['years', 'YearsView', '1999'],
			['folders', 'FolderView', 'Albums'],
			['apps', 'AppsView', ''],
		] as const) {
			const r = await open(chip);
			const panel = r.container.querySelector(`[data-library-panel="${chip}"]`)!;
			await waitFor(() => expect(panel.querySelector(`[data-view="${view}"]`)).not.toBeNull());
			if (text) await waitFor(() => expect(panel.textContent).toContain(text));
			// In place: no Back button, and the title only for screen readers (the chip says it).
			expect(panel.querySelector('.back-btn')).toBeNull();
			expect(panel.querySelector('h1')).toBeNull();
			r.unmount();
		}
		const d = await open('downloads');
		expect(d.container.querySelector('[data-library-panel="downloads"] h2')!.textContent).toBe('Downloads');
		expect(d.container.querySelector('[data-library-panel="downloads"] h1')).toBeNull();
		d.unmount();
	});

	test('Compilations: albums filtered to compilations, with their own orders', async () => {
		const r = await open('compilations');
		await waitFor(() => expect(fake.calls.some((c) => c.cmd[0] === 'albums' && c.cmd.includes('compilation:1'))).toBe(true));
		r.unmount();
	});

	test('Random is an order of Albums, with Shuffle again', async () => {
		localStorage.setItem(LIBRARY_KEY, JSON.stringify({ chip: 'albums', sort: 'random', view: 'grid' }));
		const r = render(LibraryPage);
		await waitFor(() => expect(fake.calls.filter((c) => c.cmd[0] === 'albums').length).toBe(1));
		await fireEvent.click(r.container.querySelector('[data-shuffle-again]')!);
		await waitFor(() => expect(fake.calls.filter((c) => c.cmd[0] === 'albums').length).toBe(2));
		expect(fake.calls.filter((c) => c.cmd[0] === 'albums').every((c) => c.cmd.includes('sort:random'))).toBe(true);
		r.unmount();
	});

	test('a junk remembered section falls back to Albums', async () => {
		localStorage.setItem(LIBRARY_KEY, JSON.stringify({ chip: '../settings', sort: 'album', view: 'grid' }));
		const r = render(LibraryPage);
		await tick();
		expect(r.container.querySelector('[data-library-panel]')!.getAttribute('data-library-panel')).toBe('albums');
		r.unmount();
	});
});

describe('naming', () => {
	test('"Smart playlists" wherever the user reads it', async () => {
		const r = await (async () => {
			localStorage.setItem(LIBRARY_KEY, JSON.stringify({ chip: 'filters', sort: 'album', view: 'grid' }));
			const x = render(LibraryPage);
			await tick();
			return x;
		})();
		expect(r.getByRole('tab', { name: 'Smart playlists' }).getAttribute('aria-selected')).toBe('true');
		await waitFor(() => expect(r.container.textContent).toContain('Smart playlists'));
		expect(r.container.textContent).not.toMatch(/Smart filter|Filters\b/);
		r.unmount();
	});
});
