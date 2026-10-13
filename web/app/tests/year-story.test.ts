// "Your year in sound": the full-screen story (buttons, arrow keys, swipes, Escape, focus),
// sharing the current card, and the Home card that offers it and this month's listening.
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { emptyStats, type ListeningStats } from '$lib/api/me';
import { fakeCanvas } from './helpers/fake-canvas';

vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
const state = { answers: new Map<string, ListeningStats>(), fail: false, asked: [] as string[] };
vi.mock('$lib/state/me', () => ({
	me: {
		async stats(period: string) {
			state.asked.push(period);
			if (state.fail) throw new Error('gateway down');
			return structuredClone(state.answers.get(period) ?? emptyStats(true, 396));
		},
	},
}));
const fake = await import('./helpers/fake-app.svelte');
const { default: YearStory } = await import('$lib/components/YearStory.svelte');
const { default: StatsCard } = await import('$lib/components/StatsCard.svelte');

function yearStats(): ListeningStats {
	const s = emptyStats(true, 396);
	s.totals = { plays: 412, seconds: 82800, tracks: 301, albums: 64, artists: 41, days: 27 };
	s.top_artists = [
		{ id: 1, name: 'An Example Band', plays: 40, seconds: 9100 },
		{ id: 2, name: 'Second Example', plays: 30, seconds: 8000 },
	];
	s.top_albums = [{ id: 7, plays: 22, seconds: 5200 }];
	s.top_tracks = [{ id: 9, plays: 6, seconds: 1300 }];
	s.by_hour[21] = 40;
	return s;
}

beforeEach(() => {
	vi.useFakeTimers({ toFake: ['Date'] });
	vi.setSystemTime(new Date(2026, 9, 13, 12, 0));
	state.answers = new Map([['year-2026', yearStats()]]);
	state.fail = false;
	state.asked = [];
	fake.calls.length = 0;
	fake.handler.rpc = async (_p, cmd) => {
		if (cmd[0] === 'albums') return { albums_loop: [{ id: 7, album: 'Blue Example', artist: 'An Example Band' }] };
		if (cmd[0] === 'titles') return { titles_loop: [{ id: 9, title: 'First Example Song', artist: 'An Example Band' }] };
		return {};
	};
	document.body.replaceChildren();
});
afterEach(() => vi.useRealTimers());

/** A button that opens the story, focused (focus must come back to it). */
function opener() {
	const b = document.createElement('button');
	b.textContent = 'Open';
	document.body.append(b);
	b.focus();
	return b;
}

const card = () => document.querySelector('[data-story-card]') as HTMLElement | null;
const dialog = () => document.querySelector('[data-year-story]') as HTMLDialogElement;

test('the story: one card at a time, Previous and Next buttons, no autoplay, focus on the card', async () => {
	opener();
	const onclose = vi.fn();
	render(YearStory, { year: 2026, onclose });
	await waitFor(() => expect(card()?.dataset.storyCard).toBe('hours'));
	expect(dialog().getAttribute('aria-modal')).toBe('true');
	expect(screen.getByRole('heading', { name: 'Your year in sound 2026' })).toBeTruthy();
	expect(document.activeElement).toBe(card());
	expect(card()!.getAttribute('aria-roledescription')).toBe('slide');
	expect(card()!.getAttribute('aria-label')).toBe('1 of 7');
	expect(card()!.textContent).toContain('23 hours');
	expect((screen.getByRole('button', { name: 'Previous' }) as HTMLButtonElement).disabled).toBe(true);
	// Nothing moves on its own.
	vi.useRealTimers();
	await new Promise((r) => setTimeout(r, 50));
	expect(card()!.dataset.storyCard).toBe('hours');
	const next = screen.getByRole('button', { name: 'Next' });
	next.focus();
	await fireEvent.click(next);
	expect(card()!.dataset.storyCard).toBe('artist');
	expect(card()!.textContent).toContain('An Example Band');
	await fireEvent.click(next);
	await waitFor(() => expect(card()!.textContent).toContain('Blue Example'));
	await fireEvent.click(screen.getByRole('button', { name: 'Previous' }));
	expect(card()!.dataset.storyCard).toBe('artist');
	// Back to the first card: Previous is disabled, so focus moves to Next.
	screen.getByRole('button', { name: 'Previous' }).focus();
	await fireEvent.click(screen.getByRole('button', { name: 'Previous' }));
	expect(card()!.dataset.storyCard).toBe('hours');
	expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Next' }));
	expect(onclose).not.toHaveBeenCalled();
});

test('arrow keys turn the cards; at the end Next becomes Done; Escape closes and focus goes back', async () => {
	const b = opener();
	const onclose = vi.fn();
	render(YearStory, { year: 2026, onclose });
	await waitFor(() => expect(card()).not.toBeNull());
	await fireEvent.keyDown(dialog(), { key: 'ArrowRight' });
	expect(card()!.dataset.storyCard).toBe('artist');
	await fireEvent.keyDown(dialog(), { key: 'ArrowLeft' });
	await fireEvent.keyDown(dialog(), { key: 'ArrowLeft' }); // already at the start
	expect(card()!.dataset.storyCard).toBe('hours');
	for (let i = 0; i < 10; i++) await fireEvent.keyDown(dialog(), { key: 'ArrowRight' });
	expect(card()!.dataset.storyCard).toBe('summary');
	expect(card()!.getAttribute('aria-label')).toBe('7 of 7');
	expect(screen.queryByRole('button', { name: 'Next' })).toBeNull();
	expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy();
	await fireEvent.keyDown(dialog(), { key: 'Escape' });
	expect(onclose).toHaveBeenCalledTimes(1);
	expect(document.activeElement).toBe(b);
});

test('Done and Close end the story once, returning focus', async () => {
	const b = opener();
	const onclose = vi.fn();
	render(YearStory, { year: 2026, onclose });
	await waitFor(() => expect(card()).not.toBeNull());
	await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
	await fireEvent.keyDown(dialog(), { key: 'Escape' });
	expect(onclose).toHaveBeenCalledTimes(1);
	expect(document.activeElement).toBe(b);
});

test('swipes: a sideways drag turns the card; a short or mostly vertical one does not', async () => {
	render(YearStory, { year: 2026, onclose: () => {} });
	await waitFor(() => expect(card()).not.toBeNull());
	const stage = document.querySelector('[aria-roledescription="carousel"]')!;
	const swipe = async (dx: number, dy: number) => {
		await fireEvent.pointerDown(stage, { clientX: 200, clientY: 300, pointerId: 1 });
		await fireEvent.pointerUp(stage, { clientX: 200 + dx, clientY: 300 + dy, pointerId: 1 });
	};
	await swipe(-120, 10);
	expect(card()!.dataset.storyCard).toBe('artist');
	await swipe(-30, 0);
	expect(card()!.dataset.storyCard).toBe('artist');
	await swipe(-80, 200);
	expect(card()!.dataset.storyCard).toBe('artist');
	await swipe(120, 0);
	expect(card()!.dataset.storyCard).toBe('hours');
});

test('Share makes the current card’s image on the device and hands it to the share sheet', async () => {
	const canvas = fakeCanvas();
	const share = vi.fn(async (_d: ShareData) => {});
	render(YearStory, { year: 2026, onclose: () => {}, share: { makeCanvas: () => canvas, nav: { share, canShare: () => true } } });
	await waitFor(() => expect(card()).not.toBeNull());
	await fireEvent.keyDown(dialog(), { key: 'ArrowRight' });
	await fireEvent.click(screen.getByRole('button', { name: 'Share' }));
	await waitFor(() => expect(share).toHaveBeenCalledTimes(1));
	expect(canvas.texts).toContain('Your top artist');
	expect(canvas.texts).toContain('An Example Band');
	expect(share.mock.calls[0]![0].files![0]!.name).toBe('your-year-2026-artist.png');
	expect(dialog().querySelector('.status')!.textContent).toBe('Shared.');
	// Made locally: no request left the device for it.
	expect(fake.calls.every((c) => c.cmd[0] === 'albums' || c.cmd[0] === 'titles')).toBe(true);
});

test('Share without file sharing saves the PNG and says nothing was uploaded', async () => {
	const createObjectURL = vi.fn(() => 'blob:local');
	const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
	render(YearStory, { year: 2026, onclose: () => {}, share: { makeCanvas: fakeCanvas, nav: null, url: { createObjectURL, revokeObjectURL: () => {} } } });
	await waitFor(() => expect(card()).not.toBeNull());
	await fireEvent.click(screen.getByRole('button', { name: 'Share' }));
	await waitFor(() => expect(dialog().querySelector('.status')!.textContent).toContain('nothing was uploaded'));
	expect(click).toHaveBeenCalledTimes(1);
	click.mockRestore();
});

test('too little history: says how many more days are needed; switched off: says so', async () => {
	const few = yearStats();
	few.totals.days = 4;
	state.answers.set('year-2026', few);
	const r = render(YearStory, { year: 2026, onclose: () => {} });
	await waitFor(() => expect(dialog().textContent).toContain('3 more days with music to go'));
	expect(card()).toBeNull();
	r.unmount();
	state.answers.set('year-2026', emptyStats(false, 396));
	render(YearStory, { year: 2026, onclose: () => {} });
	await waitFor(() => expect(dialog().textContent).toContain('Listening history is switched off'));
});

test('Home card: this month’s hours and plays, linking to Your listening', async () => {
	const m = yearStats();
	state.answers.set('month-2026-10', m);
	render(StatsCard);
	const link = await screen.findByRole('link', { name: /This month/ });
	expect(link.textContent).toContain('This month: 23 h · 412 plays');
	expect(link.getAttribute('href')).toBe('#/stats');
	// Not December or January: the story isn't offered unasked, and isn't fetched.
	expect(screen.queryByRole('button', { name: /Your year in sound/ })).toBeNull();
	expect(state.asked).toEqual(['month-2026-10']);
});

test('Home card: hidden when history is off, empty, or the stats fail', async () => {
	for (const answer of [emptyStats(false, 396), emptyStats(true, 396), null]) {
		state.fail = answer === null;
		if (answer) state.answers.set('month-2026-10', answer);
		const r = render(StatsCard);
		await waitFor(() => expect(state.asked.length).toBeGreaterThan(0));
		await new Promise((res) => queueMicrotask(() => res(null)));
		expect(r.container.querySelector('[data-stats-card]')).toBeNull();
		r.unmount();
		state.asked = [];
	}
});

test('Home card in December: offers Your year in sound, which opens the story', async () => {
	vi.setSystemTime(new Date(2026, 11, 3, 9, 0));
	state.answers.set('month-2026-12', yearStats());
	render(StatsCard);
	const offer = await screen.findByRole('button', { name: 'Your year in sound 2026' });
	expect(state.asked.sort()).toEqual(['month-2026-12', 'year-2026']);
	offer.focus();
	await fireEvent.click(offer);
	await waitFor(() => expect(card()?.dataset.storyCard).toBe('hours'));
	await fireEvent.keyDown(dialog(), { key: 'Escape' });
	await waitFor(() => expect(document.querySelector('[data-year-story]')).toBeNull());
	expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Your year in sound 2026' }));
});

test('Home card in January: the year just ended, even before this month has plays', async () => {
	vi.setSystemTime(new Date(2027, 0, 1, 9, 0));
	state.answers.set('month-2027-01', emptyStats(true, 396));
	render(StatsCard);
	expect(await screen.findByRole('button', { name: 'Your year in sound 2026' })).toBeTruthy();
	expect(screen.queryByRole('link', { name: /This month/ })).toBeNull();
});
