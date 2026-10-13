// Keyboard shortcuts (wave2-design.md §5.5): Space plays or pauses, `/` goes to search, `L`
// shows the lyrics, `?` lists the shortcuts; on Now playing and in car mode (which don't
// scroll) the arrow keys seek, skip and change the volume. They never fire while typing, in a
// dialog or menu, with Ctrl, ⌘ or Alt held, or (Space and arrows) on a control that uses the
// key itself (a button, a link, a slider, tabs…).

export type Shortcut = 'playpause' | 'seekback' | 'seekforward' | 'previous' | 'next' | 'volumeup' | 'volumedown' | 'search' | 'lyrics' | 'help';

/** Seconds moved by ← and →; volume steps of ↑ and ↓. */
export const SEEK_STEP = 10;
export const VOLUME_STEP = 5;

/** What the help sheet lists, in order. */
export const SHORTCUT_HELP: readonly { keys: string[]; label: string; where?: string }[] = [
	{ keys: ['Space'], label: 'Play or pause' },
	{ keys: ['←', '→'], label: `Back or forward ${SEEK_STEP} seconds`, where: 'Now playing and car mode' },
	{ keys: ['Shift', '←', '→'], label: 'Previous or next track', where: 'Now playing and car mode' },
	{ keys: ['↑', '↓'], label: 'Volume up or down', where: 'Now playing and car mode' },
	{ keys: ['/'], label: 'Search' },
	{ keys: ['L'], label: 'Lyrics' },
	{ keys: ['1', '2', '3'], label: 'Art, Lyrics or Info layout', where: 'Now playing' },
	{ keys: ['?'], label: 'This list of shortcuts' },
];

const TYPING = 'input, textarea, select, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"]';
const MODAL = 'dialog, [role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [aria-modal="true"]';
const USES_KEYS =
	'button, a[href], summary, audio, video, [role="button"], [role="link"], [role="slider"], [role="tab"], [role="checkbox"], [role="switch"], [role="radio"], [role="menuitem"], [role="option"], [role="spinbutton"], [role="scrollbar"], [role="treeitem"], [role="gridcell"]';

const asElement = (t: EventTarget | null): Element | null => (t && typeof (t as Element).closest === 'function' ? (t as Element) : null);

/** The user is typing (a text field, a select, an editable region). */
export function typing(target: EventTarget | null): boolean {
	return !!asElement(target)?.closest(TYPING);
}

/** In a dialog or menu, or a modal dialog is open anywhere. */
export function inDialog(target: EventTarget | null, doc: Document | null = typeof document !== 'undefined' ? document : null): boolean {
	if (asElement(target)?.closest(MODAL)) return true;
	return !!doc?.querySelector('dialog[open]');
}

/** A control that uses Space or the arrows itself. */
export function usesKeys(target: EventTarget | null): boolean {
	return !!asElement(target)?.closest(USES_KEYS);
}

export interface KeyLike {
	key: string;
	target: EventTarget | null;
	shiftKey?: boolean;
	ctrlKey?: boolean;
	metaKey?: boolean;
	altKey?: boolean;
	repeat?: boolean;
	isComposing?: boolean;
	defaultPrevented?: boolean;
}

/** The shortcut for a key press, or null. `arrows`: the page lets the arrow keys act. */
export function shortcutFor(e: KeyLike, ctx: { arrows: boolean; doc?: Document | null }): Shortcut | null {
	if (e.defaultPrevented || e.isComposing || e.ctrlKey || e.metaKey || e.altKey) return null;
	if (typing(e.target) || inDialog(e.target, ctx.doc)) return null;
	const k = e.key;
	if (k === ' ' || k === 'Spacebar') return e.repeat || e.shiftKey || usesKeys(e.target) ? null : 'playpause';
	if (k === 'ArrowLeft' || k === 'ArrowRight' || k === 'ArrowUp' || k === 'ArrowDown') {
		if (!ctx.arrows || usesKeys(e.target)) return null;
		if (k === 'ArrowUp') return e.shiftKey ? null : 'volumeup';
		if (k === 'ArrowDown') return e.shiftKey ? null : 'volumedown';
		if (e.shiftKey) return e.repeat ? null : k === 'ArrowLeft' ? 'previous' : 'next';
		return k === 'ArrowLeft' ? 'seekback' : 'seekforward';
	}
	if (e.repeat) return null;
	if (k === '/') return 'search';
	if (k === 'l' || k === 'L') return 'lyrics';
	if (k === '?') return 'help';
	return null;
}
