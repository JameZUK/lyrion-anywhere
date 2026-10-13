// Shared test set-up. SvelteKit's client router doesn't run under jsdom, so `goto` is a spy
// that records where the app went (and moves the address like the router would, without the
// `hashchange` a hand-made `location.hash` assignment fires).
vi.mock('$app/navigation', () => ({
	goto: vi.fn(async (url: string) => {
		history.pushState(history.state, '', url);
	}),
	afterNavigate: vi.fn(),
	beforeNavigate: vi.fn(),
	onNavigate: vi.fn(),
	invalidate: vi.fn(async () => {}),
	invalidateAll: vi.fn(async () => {}),
	pushState: vi.fn(),
	replaceState: vi.fn(),
}));
