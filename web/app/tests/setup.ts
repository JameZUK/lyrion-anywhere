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

// The tests of a file share one process, and several build the app afresh: Node's real
// BroadcastChannel would carry one test's sign-out to the app instances of the others (and
// keep the process alive). The bus between tabs is tested with its own fakes
// (session-identity.test.ts); here the app finds no channel, as in a browser without one.
Object.defineProperty(globalThis, 'BroadcastChannel', { value: undefined, configurable: true, writable: true });

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
