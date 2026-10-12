// The mini player's gestures (docs/plans: Wave 1 §4), the same on web, Android and iOS:
//
// | Gesture             | Action                         |
// |---------------------|--------------------------------|
// | Swipe left / right  | Next / previous track          |
// | Swipe up            | Open Now playing               |
// | Long-press          | Play on… (the player picker)   |
// | Tap                 | Open Now playing               |
//
// A small state machine, free of the DOM: the component feeds it pointer positions and
// times, and acts on what it returns. Every gesture also has a button, a keyboard shortcut
// and a screen-reader action. The Android app mirrors these rules (ui/player/Gestures.kt).

export type MiniAction = 'tap' | 'long-press' | 'next' | 'previous' | 'open' | 'none';

/** Movement (px) within which a press is still a tap or a long-press. */
export const SLOP_PX = 10;
/** Held this long without moving: a long-press. */
export const LONG_PRESS_MS = 500;
/** A sideways swipe must travel this far, or this share of the bar's width (the larger). */
export const SWIPE_MIN_PX = 60;
export const SWIPE_SHARE = 0.2;
/** An upward swipe must travel this far. */
export const SWIPE_UP_PX = 40;
/** How much more along its axis than across it a swipe must move. */
export const AXIS_RATIO = 1.5;

type State =
	| { kind: 'idle' }
	| { kind: 'pressed'; x: number; y: number; t: number }
	| { kind: 'dragging'; axis: 'x' | 'y'; x: number; y: number; dx: number; dy: number }
	/** The long-press fired: the rest of this press does nothing. */
	| { kind: 'done' };

export class MiniGesture {
	#s: State = { kind: 'idle' };

	constructor(private width = 0) {}

	/** The bar's width (px), for the swipe distance. */
	setWidth(w: number) {
		this.width = Number.isFinite(w) && w > 0 ? w : 0;
	}

	get state(): State['kind'] {
		return this.#s.kind;
	}

	/** While dragging: how far, and along which axis (for following the finger). */
	get drag(): { axis: 'x' | 'y'; dx: number; dy: number } | null {
		const s = this.#s;
		return s.kind === 'dragging' ? { axis: s.axis, dx: s.dx, dy: s.dy } : null;
	}

	down(x: number, y: number, t: number) {
		if (![x, y, t].every(Number.isFinite)) return;
		this.#s = { kind: 'pressed', x, y, t };
	}

	/** Pointer moved. Returns true once the press has become a drag (the caller then stops scrolling). */
	move(x: number, y: number): boolean {
		const s = this.#s;
		if (!Number.isFinite(x) || !Number.isFinite(y)) return false;
		if (s.kind === 'pressed') {
			const dx = x - s.x;
			const dy = y - s.y;
			if (Math.hypot(dx, dy) <= SLOP_PX) return false;
			if (Math.abs(dx) > Math.abs(dy) * AXIS_RATIO) this.#s = { kind: 'dragging', axis: 'x', x: s.x, y: s.y, dx, dy };
			else if (Math.abs(dy) > Math.abs(dx) * AXIS_RATIO) this.#s = { kind: 'dragging', axis: 'y', x: s.x, y: s.y, dx, dy };
			else this.#s = { kind: 'done' }; // diagonal: not ours
			return this.#s.kind === 'dragging';
		}
		if (s.kind === 'dragging') {
			this.#s = { ...s, dx: x - s.x, dy: y - s.y };
			return true;
		}
		return false;
	}

	/**
	 * Time passing while held (call from a timer): returns 'long-press' once, when the press
	 * has been held still long enough.
	 */
	tick(t: number): MiniAction {
		const s = this.#s;
		if (s.kind === 'pressed' && t - s.t >= LONG_PRESS_MS) {
			this.#s = { kind: 'done' };
			return 'long-press';
		}
		return 'none';
	}

	/** Pointer lifted: what the gesture asked for. */
	up(x: number, y: number, t: number): MiniAction {
		const s = this.#s;
		this.#s = { kind: 'idle' };
		if (s.kind === 'pressed') {
			if (Number.isFinite(t) && t - s.t >= LONG_PRESS_MS) return 'long-press';
			return 'tap';
		}
		if (s.kind !== 'dragging') return 'none';
		const dx = Number.isFinite(x) ? x - s.x : s.dx;
		const dy = Number.isFinite(y) ? y - s.y : s.dy;
		return classifySwipe(s.axis, dx, dy, this.width);
	}

	/** The pointer was taken away (scrolling, the window lost focus). */
	cancel() {
		this.#s = { kind: 'idle' };
	}
}

/** What a finished drag along `axis` asks for. */
export function classifySwipe(axis: 'x' | 'y', dx: number, dy: number, width: number): MiniAction {
	if (!Number.isFinite(dx) || !Number.isFinite(dy)) return 'none';
	if (axis === 'x') {
		const w = Number.isFinite(width) && width > 0 ? width : 0;
		const need = Math.max(SWIPE_MIN_PX, w * SWIPE_SHARE);
		if (dx <= -need) return 'next';
		if (dx >= need) return 'previous';
		return 'none';
	}
	return dy <= -SWIPE_UP_PX ? 'open' : 'none';
}

/**
 * Keyboard shortcuts on the focused mini player: Enter or Space opens Now playing,
 * Alt+→ / Alt+← next / previous, and the context-menu key or Shift+F10 opens Play on….
 */
export function miniKey(e: { key: string; altKey?: boolean; shiftKey?: boolean; ctrlKey?: boolean; metaKey?: boolean }): MiniAction {
	if (e.ctrlKey || e.metaKey) return 'none';
	if (e.altKey && e.key === 'ArrowRight') return 'next';
	if (e.altKey && e.key === 'ArrowLeft') return 'previous';
	if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) return 'long-press';
	if (!e.altKey && !e.shiftKey && (e.key === 'Enter' || e.key === ' ')) return 'open';
	return 'none';
}
