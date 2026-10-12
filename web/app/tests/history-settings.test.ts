// Settings › Listening history: switch it off, view it (with titles), clear it.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { HistoryEntry } from '$lib/api/me';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const state = { enabled: true, items: [] as HistoryEntry[], next: null as string | null, patches: [] as boolean[], cleared: 0 };
vi.mock('$lib/state/me', () => ({
	me: {
		async history(_limit: number, before?: string | null) {
			return before
				? { enabled: state.enabled, retention_days: 396, items: [entry(3, 1_700_000_000)], next: null }
				: { enabled: state.enabled, retention_days: 396, items: state.items, next: state.next };
		},
		async setHistory(on: boolean) {
			state.patches.push(on);
			state.enabled = on;
			return { enabled: on };
		},
		async clearHistory() {
			state.cleared++;
			const n = state.items.length;
			state.items = [];
			return { removed: n };
		},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { default: HistorySettings } = await import('$lib/components/HistorySettings.svelte');

function entry(track: number, at: number): HistoryEntry {
	return { track_key: 'a'.repeat(64), track_id: track, album_id: 7, artist_id: 2, year: 1984, played_at: at, seconds: 200, source: track === 2 ? 'offline' : 'live' };
}

beforeEach(() => {
	state.enabled = true;
	state.items = [entry(1, 1_790_000_000), entry(2, 1_789_000_000), entry(99, 1_788_000_000)];
	state.next = '1788000000-3';
	state.patches = [];
	state.cleared = 0;
	fake.calls.length = 0;
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'titles') return { titles_loop: [{ id: 1, title: 'First Song', artist: 'Band' }, { id: 2, title: 'Second Song', artist: 'Band' }, { id: 3, title: 'Older Song', artist: 'Band' }] };
		return {};
	};
});

test('says how long it is kept and that it is never shared', async () => {
	render(HistorySettings);
	expect(document.body.textContent).toContain('13 months');
	expect(document.body.textContent).toContain('never shared');
	await waitFor(() => expect((screen.getByRole('checkbox', { name: 'Keep my listening history' }) as HTMLInputElement).checked).toBe(true));
});

test('switching it off is sent and confirmed', async () => {
	render(HistorySettings);
	const box = await screen.findByRole('checkbox', { name: 'Keep my listening history' });
	await fireEvent.click(box);
	await waitFor(() => expect(state.patches).toEqual([false]));
	expect(screen.getByRole('status').textContent).toContain('nothing more is recorded');
});

test('the history lists titles by track id, older pages on request', async () => {
	render(HistorySettings);
	await fireEvent.click(screen.getByRole('button', { name: 'View history' }));
	const list = await screen.findByRole('list', { name: 'Listening history' });
	await waitFor(() => expect(list.textContent).toContain('First Song · Band'));
	expect(list.textContent).toContain('offline');
	expect(list.textContent).toContain('A track no longer in the library');
	const titles = fake.calls.find((c) => c.cmd[0] === 'titles')!;
	expect(titles.cmd.at(-1)).toBe('track_id:1,2,99');
	await fireEvent.click(screen.getByRole('button', { name: 'Show older' }));
	await waitFor(() => expect(list.textContent).toContain('Older Song'));
	expect(screen.queryByRole('button', { name: 'Show older' })).toBeNull();
});

test('clearing asks first', async () => {
	render(HistorySettings);
	vi.spyOn(window, 'confirm').mockReturnValueOnce(false);
	await fireEvent.click(screen.getByRole('button', { name: 'Clear history' }));
	expect(state.cleared).toBe(0);
	vi.spyOn(window, 'confirm').mockReturnValueOnce(true);
	await fireEvent.click(screen.getByRole('button', { name: 'Clear history' }));
	await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Cleared 3 plays.'));
});
