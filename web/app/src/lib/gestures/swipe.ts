// Sideways swipes on list rows (Wave 2 §3.4), Squeezer style: touch and pen only (a mouse
// drags text, as people expect), vertical movement stays the browser's scroll, and a swipe
// acts only once it has gone far enough (see swipeAction). A swipe is never the only way to
// do something: every row also has a menu and keyboard shortcuts.
import { isSwipe, swipeAction } from '#lib/lms/queue-edit';

export interface SwipeOptions {
	onleft?: () => void;
	onright?: () => void;
}

/** Svelte action: `use:swipeRow={{ onleft, onright }}`. */
export function swipeRow(node: HTMLElement, options: SwipeOptions) {
	let opts = options;
	let s: { id: number; x: number; y: number; dx: number; active: boolean; width: number } | null = null;
	let swallow = false;
	node.style.touchAction = 'pan-y';

	const clamp = (dx: number) => Math.max(opts.onleft ? -Infinity : 0, Math.min(opts.onright ? Infinity : 0, dx));
	const settle = () => {
		node.style.transform = '';
		node.removeAttribute('data-swiping');
	};

	function down(e: PointerEvent) {
		if (e.pointerType === 'mouse' || s) return;
		s = { id: e.pointerId, x: e.clientX, y: e.clientY, dx: 0, active: false, width: node.getBoundingClientRect().width };
	}
	function move(e: PointerEvent) {
		if (!s || e.pointerId !== s.id) return;
		const dx = e.clientX - s.x;
		const dy = e.clientY - s.y;
		if (!s.active) {
			if (!isSwipe(dx, dy)) {
				if (Math.abs(dy) > 12) s = null; // a scroll: not ours
				return;
			}
			s.active = true;
			node.setPointerCapture?.(e.pointerId);
		}
		s.dx = clamp(dx);
		node.style.transform = `translateX(${s.dx}px)`;
		node.setAttribute('data-swiping', s.dx < 0 ? 'left' : 'right');
	}
	function up(e: PointerEvent, cancelled = false) {
		if (!s || e.pointerId !== s.id) return;
		const { dx, active, width } = s;
		s = null;
		settle();
		if (!active) return;
		swallow = true;
		setTimeout(() => (swallow = false), 400);
		if (cancelled) return;
		const way = swipeAction(dx, width);
		if (way === 'left') opts.onleft?.();
		else if (way === 'right') opts.onright?.();
	}
	const onup = (e: PointerEvent) => up(e);
	const oncancel = (e: PointerEvent) => up(e, true);
	/** The tap that ends a swipe doesn't also open or play the row. */
	function click(e: MouseEvent) {
		if (!swallow) return;
		swallow = false;
		e.preventDefault();
		e.stopPropagation();
	}

	node.addEventListener('pointerdown', down);
	node.addEventListener('pointermove', move);
	node.addEventListener('pointerup', onup);
	node.addEventListener('pointercancel', oncancel);
	node.addEventListener('click', click, true);
	return {
		update(o: SwipeOptions) {
			opts = o;
		},
		destroy() {
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointermove', move);
			node.removeEventListener('pointerup', onup);
			node.removeEventListener('pointercancel', oncancel);
			node.removeEventListener('click', click, true);
		},
	};
}

/**
 * Keyboard alternatives on a focused browse row: N plays next, A adds to the queue (no
 * modifier keys, so typing elsewhere is never caught). Returns whether the key was used.
 */
export function rowShortcut(e: KeyboardEvent, actions: { next?: () => void; add?: () => void; remove?: () => void }): boolean {
	if (e.altKey || e.ctrlKey || e.metaKey) return false;
	const k = e.key.toLowerCase();
	const run = k === 'n' ? actions.next : k === 'a' ? actions.add : e.key === 'Delete' || e.key === 'Backspace' ? actions.remove : undefined;
	if (!run) return false;
	e.preventDefault();
	run();
	return true;
}
