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

// jsdom may lack modal dialogs: enough of them for components that use <dialog>.
if (typeof HTMLDialogElement !== 'undefined') {
	HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
		this.setAttribute('open', '');
	};
	HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
		this.removeAttribute('open');
		this.dispatchEvent(new Event('close'));
	};
}
