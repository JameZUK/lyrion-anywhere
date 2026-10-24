// The notice on a page served over plain http:// to anything but loopback (a trusted network,
// SPEC D28). Browsers treat such a page as an insecure context and withhold the service
// worker, the Cache API, storage persistence, Web Locks and AudioWorklet: the app still
// works (the media-element player, IndexedDB downloads), but installing it, offline use and
// some player features need HTTPS. Dismissed for the rest of the browser session.
import { isLoopback } from '#lib/qr/qr.js';

export const INSECURE_TEXT = 'Unencrypted connection. Installing the app, offline use and some player features need HTTPS.';
/** sessionStorage key: the notice was dismissed in this browser session. */
export const INSECURE_DISMISSED_KEY = 'anywhere.insecureNoticeDismissed';

export interface PageOrigin {
	protocol?: string;
	hostname?: string;
	isSecureContext?: boolean;
}

/**
 * The notice to show for a page at this origin, or null. Only plain `http:` counts (not
 * `https:`, `file:` or anything else), never for loopback (browsers treat localhost as
 * secure), and never when the browser itself says the context is secure.
 */
export function insecureContextNotice(o: PageOrigin): string | null {
	if (o.protocol !== 'http:') return null;
	if (typeof o.hostname !== 'string' || !o.hostname || isLoopback(o.hostname)) return null;
	if (o.isSecureContext === true) return null;
	return INSECURE_TEXT;
}

type SessionLike = Pick<Storage, 'getItem' | 'setItem'>;

function browserSession(): SessionLike | undefined {
	try {
		return globalThis.sessionStorage;
	} catch {
		return undefined;
	}
}

function browserOrigin(): PageOrigin {
	const l = globalThis.location as Location | undefined;
	return { protocol: l?.protocol, hostname: l?.hostname, isSecureContext: (globalThis as { isSecureContext?: boolean }).isSecureContext };
}

export class InsecureNotice {
	/** What the notice says, or null when this page needs none. */
	readonly text: string | null;
	dismissed = $state(false);
	#storage: SessionLike | null | undefined;

	constructor(origin: PageOrigin = browserOrigin(), storage: SessionLike | null | undefined = browserSession()) {
		this.text = insecureContextNotice(origin);
		this.#storage = storage;
		try {
			this.dismissed = storage?.getItem(INSECURE_DISMISSED_KEY) === '1';
		} catch {
			this.dismissed = false;
		}
	}

	get shown(): boolean {
		return this.text !== null && !this.dismissed;
	}

	/** Hide it for this session (or, without sessionStorage, for this page load). */
	dismiss() {
		this.dismissed = true;
		try {
			this.#storage?.setItem(INSECURE_DISMISSED_KEY, '1');
		} catch {
			/* this page load only */
		}
	}
}

export const insecureNotice = new InsecureNotice();
