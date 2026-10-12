// The mini player's gesture model (docs/plans: Wave 1 §4): swipe left/right for next/previous,
// up to open, long-press for Play on…, tap to open; and the keyboard equivalents.
import { LONG_PRESS_MS, MiniGesture, SLOP_PX, SWIPE_MIN_PX, classifySwipe, miniKey } from '$lib/player/gesture';

function swipe(g: MiniGesture, dx: number, dy: number, steps = 5) {
	g.down(100, 100, 0);
	for (let i = 1; i <= steps; i++) g.move(100 + (dx * i) / steps, 100 + (dy * i) / steps);
	return g.up(100 + dx, 100 + dy, 200);
}

test('a quick touch is a tap; held still, a long-press', () => {
	const g = new MiniGesture(360);
	g.down(10, 10, 0);
	expect(g.up(12, 11, 120)).toBe('tap');
	// Held: the timer's tick fires the long-press once, and the lift then does nothing.
	g.down(10, 10, 0);
	expect(g.tick(LONG_PRESS_MS - 1)).toBe('none');
	expect(g.tick(LONG_PRESS_MS)).toBe('long-press');
	expect(g.tick(LONG_PRESS_MS + 500)).toBe('none');
	expect(g.up(10, 10, LONG_PRESS_MS + 600)).toBe('none');
	// No timer ran (a throttled page): the lift still counts it.
	g.down(10, 10, 0);
	expect(g.up(10, 10, LONG_PRESS_MS + 10)).toBe('long-press');
});

test('swipes: left for next, right for previous, up to open', () => {
	const g = new MiniGesture(360);
	expect(swipe(g, -100, 5)).toBe('next');
	expect(swipe(g, 100, -5)).toBe('previous');
	expect(swipe(g, 0, -60)).toBe('open');
	// Too short: nothing.
	expect(swipe(g, -40, 0)).toBe('none');
	expect(swipe(g, 0, -20)).toBe('none');
	// Downwards does nothing on the mini player.
	expect(swipe(g, 0, 80)).toBe('none');
	// A wide bar needs a longer swipe (a share of its width).
	const wide = new MiniGesture(1000);
	expect(swipe(wide, -150, 0)).toBe('none');
	expect(swipe(wide, -220, 0)).toBe('next');
});

test('a diagonal or a wobble is not a gesture, and moving cancels the long-press', () => {
	const g = new MiniGesture(360);
	expect(swipe(g, 80, -80)).toBe('none');
	g.down(0, 0, 0);
	expect(g.move(SLOP_PX - 1, 0)).toBe(false);
	expect(g.state).toBe('pressed');
	expect(g.up(SLOP_PX - 1, 0, 100)).toBe('tap');
	g.down(0, 0, 0);
	expect(g.move(-30, 0)).toBe(true);
	expect(g.tick(LONG_PRESS_MS * 2)).toBe('none');
	expect(g.drag).toEqual({ axis: 'x', dx: -30, dy: 0 });
	g.cancel();
	expect(g.state).toBe('idle');
	expect(g.up(-200, 0, 100)).toBe('none'); // cancelled: the lift does nothing
});

test('the axis is decided once: a sideways swipe that drifts up stays sideways', () => {
	const g = new MiniGesture(360);
	g.down(100, 100, 0);
	g.move(70, 102); // sideways first
	g.move(20, 40); // then mostly upwards
	expect(g.up(20, 40, 300)).toBe('next');
});

test('classification at the thresholds', () => {
	expect(classifySwipe('x', -SWIPE_MIN_PX, 0, 0)).toBe('next');
	expect(classifySwipe('x', -SWIPE_MIN_PX + 1, 0, 0)).toBe('none');
	expect(classifySwipe('x', SWIPE_MIN_PX, 0, 0)).toBe('previous');
	expect(classifySwipe('y', 0, -40, 0)).toBe('open');
	expect(classifySwipe('y', 0, -39, 0)).toBe('none');
	expect(classifySwipe('x', NaN, 0, 300)).toBe('none');
});

test('arbitrary input never throws and always ends idle (robustness)', () => {
	let seed = 7;
	const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31) * 2 - 1;
	const odd = [NaN, Infinity, -Infinity, 1e308, -0];
	const pick = () => (rnd() > 0.9 ? odd[Math.floor(Math.abs(rnd()) * odd.length)]! : rnd() * 2000);
	const g = new MiniGesture(pick());
	for (let i = 0; i < 5000; i++) {
		const op = Math.abs(rnd());
		expect(() => {
			if (op < 0.2) g.down(pick(), pick(), pick());
			else if (op < 0.6) g.move(pick(), pick());
			else if (op < 0.7) g.tick(pick());
			else if (op < 0.75) g.setWidth(pick());
			else if (op < 0.8) g.cancel();
			else expect(['tap', 'long-press', 'next', 'previous', 'open', 'none']).toContain(g.up(pick(), pick(), pick()));
		}).not.toThrow();
	}
	g.up(0, 0, 0);
	expect(g.state).toBe('idle');
});

test('keyboard equivalents', () => {
	expect(miniKey({ key: 'ArrowRight', altKey: true })).toBe('next');
	expect(miniKey({ key: 'ArrowLeft', altKey: true })).toBe('previous');
	expect(miniKey({ key: 'ContextMenu' })).toBe('long-press');
	expect(miniKey({ key: 'F10', shiftKey: true })).toBe('long-press');
	expect(miniKey({ key: 'Enter' })).toBe('open');
	expect(miniKey({ key: ' ' })).toBe('open');
	// Browser and system shortcuts are left alone.
	expect(miniKey({ key: 'ArrowRight' })).toBe('none');
	expect(miniKey({ key: 'ArrowLeft', altKey: true, ctrlKey: true })).toBe('none');
	expect(miniKey({ key: 'Enter', metaKey: true })).toBe('none');
	expect(miniKey({ key: 'F10' })).toBe('none');
});
