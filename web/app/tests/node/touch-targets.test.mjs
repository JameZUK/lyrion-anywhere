// Touch targets: controls are at least 44 × 44 CSS px on touch screens (WCAG 2.5.5). jsdom has
// no layout, so the styles that guarantee it are checked here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (p) => readFileSync(new URL(`../../src/${p}`, import.meta.url), 'utf8');
/** The declarations of the first rule whose selector list contains `selector` (after `from`). */
function rule(css, selector, from = 0) {
	const at = css.indexOf(selector, from);
	assert.ok(at >= 0, `no rule for ${selector}`);
	const open = css.indexOf('{', at);
	return css.slice(open + 1, css.indexOf('}', open));
}
const px = (decls, prop) => Number(decls.match(new RegExp(`(?:^|[;\\s])${prop}:\\s*(\\d+)px`))?.[1] ?? NaN);

test('icon buttons and buttons are 44 px on touch screens', () => {
	const css = read('app.css');
	const coarse = css.indexOf('@media (pointer: coarse)');
	assert.ok(coarse >= 0);
	const icon = rule(css, '.icon-btn', coarse);
	assert.ok(px(icon, 'width') >= 44 && px(icon, 'height') >= 44, icon);
	assert.ok(px(rule(css, '.btn', coarse), 'min-height') >= 44);
	assert.ok(px(rule(css, '.back-btn'), 'min-height') >= 44);
});

test('rating stars and the heart are 44 px targets', () => {
	const d = rule(read('lib/components/RatingControl.svelte'), '.heart {');
	assert.ok(px(d, 'min-width') >= 44 && px(d, 'min-height') >= 44, d);
});

test('Wave 2 library and queue controls keep 44 px targets', () => {
	const chip = rule(read('lib/components/Chips.svelte'), 'button {');
	assert.ok(px(chip, 'min-height') >= 44, chip);
	assert.ok(px(rule(read('lib/components/RowMenu.svelte'), '.menu button {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/ShuffleButton.svelte'), '.menu button {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/TrackCredits.svelte'), '\ta {'), 'min-height') >= 44);
	assert.ok(px(rule(read('routes/artist/[id]/[[role]]/+page.svelte'), '.chip {'), 'min-height') >= 44);
	assert.ok(px(rule(read('routes/search/+page.svelte'), '.term {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/SearchHit.svelte'), '.main {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/QueueList.svelte'), 'li button {'), 'min-height') >= 44);
	assert.ok(px(rule(read('routes/library/+page.svelte'), '.browse a {'), 'min-height') >= 44);
	// The A–Z strip is 44 px wide; dragging along it picks a letter (and the keyboard reaches each).
	assert.ok(px(rule(read('lib/components/AzScroller.svelte'), '.strip {'), 'width') >= 44);
});

test('Up next, the sleep timer and track menus keep 44 px targets', () => {
	const rm = rule(read('lib/components/UpNext.svelte'), '.rm {');
	assert.ok(px(rm, 'width') >= 44 && px(rm, 'height') >= 44);
	const cancel = rule(read('routes/+page.svelte'), '.cancel {');
	assert.ok(px(cancel, 'width') >= 44 && px(cancel, 'height') >= 44);
	assert.ok(px(rule(read('lib/components/TrackList.svelte'), '.menu button {'), 'min-height') >= 44);
});

test('Now playing layouts, the visualiser and Rooms keep 44 px targets', () => {
	assert.ok(px(rule(read('routes/+page.svelte'), '.segbtn {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/LyricsLines.svelte'), '.line {'), 'min-height') >= 44);
});
