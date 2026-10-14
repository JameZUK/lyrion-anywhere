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
	// Up next is one line, the whole of it a 44 px tall link (no remove button since Wave 3.5).
	assert.ok(px(rule(read('lib/components/UpNext.svelte'), '.row {'), 'min-height') >= 44);
	// Sleep timer and its off switch are items of Now playing's ⋯ (RowMenu, 44 px rows).
	assert.match(read('routes/+page.svelte'), /<RowMenu label="More options"/);
	assert.ok(px(rule(read('lib/components/TrackList.svelte'), '.menu button {'), 'min-height') >= 44);
});

test('Wave 3.5: the one switch, Play on…, the queue handle and menus keep 44 px targets', () => {
	assert.ok(px(rule(read('lib/components/Switch.svelte'), '.switch {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/VolumeSlider.svelte'), 'input {'), 'height') >= 44);
	assert.ok(px(rule(read('lib/components/FormatPicker.svelte'), '.row {'), 'min-height') >= 44);
	const queue = read('routes/queue/+page.svelte');
	const coarse = queue.indexOf('@media (pointer: coarse)');
	assert.ok(coarse >= 0);
	assert.ok(px(rule(queue, '.handle {', coarse), 'width') >= 44);
	assert.ok(px(rule(read('lib/components/ContinueOn.svelte'), '.row {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/Snackbar.svelte'), '.act {'), 'min-height') >= 44);
	assert.ok(px(rule(read('routes/home/+page.svelte'), '.edit-home {'), 'min-height') >= 44);
});

test('Now playing layouts, the visualiser and Rooms keep 44 px targets', () => {
	assert.ok(px(rule(read('routes/+page.svelte'), '.segbtn {'), 'min-height') >= 44);
	assert.ok(px(rule(read('lib/components/LyricsLines.svelte'), '.line {'), 'min-height') >= 44);
	const card = read('lib/components/RoomCard.svelte');
	const icon = rule(card, '.icon-btn {');
	assert.ok(px(icon, 'width') >= 44 && px(icon, 'height') >= 44, icon);
	assert.ok(px(rule(card, '.vol {'), 'min-height') >= 44);
	assert.ok(px(rule(card, '.main {'), 'min-height') >= 44);
	assert.ok(px(rule(read('routes/rooms/+page.svelte'), '.choose .btn {'), 'min-height') >= 44);
});

test('smart filters keep 44 px targets', () => {
	// A row's one trailing control is its ⋯ (RowMenu's icon button: 44 px on touch).
	assert.match(read('lib/components/FiltersView.svelte'), /<RowMenu/);
	assert.ok(px(rule(read('lib/components/FiltersView.svelte'), '.main {'), 'min-height') >= 44);
	const editor = read('lib/components/FilterEditor.svelte');
	assert.ok(px(rule(editor, '.check {'), 'min-height') >= 44);
	assert.ok(px(rule(editor, '.actions .btn {'), 'min-height') >= 44);
	assert.ok(px(rule(editor, '.field input {'), 'min-height') >= 44);
	const shelf = rule(read('lib/components/HomeShelf.svelte'), '.head .icon-btn {');
	assert.ok(px(shelf, 'width') >= 44 && px(shelf, 'height') >= 44, shelf);
	assert.ok(px(rule(read('routes/filter/[id]/+page.svelte'), '.actions .btn {'), 'min-height') >= 44);
});

test('car mode: every target at least 76 px', () => {
	const car = read('routes/car/+page.svelte');
	const big = rule(car, '.big {');
	assert.ok(px(big, 'min-width') >= 76 && px(big, 'min-height') >= 76, big);
	const exit = rule(car, '.exit {');
	assert.ok(px(exit, 'min-width') >= 76 && px(exit, 'min-height') >= 76, exit);
	assert.ok(px(rule(car, '.wide {'), 'min-height') >= 76);
	assert.ok(px(rule(car, '.item {'), 'min-height') >= 76);
	// High contrast in every theme: its own black and white, not the theme's colours.
	const root = rule(car, '.car {');
	assert.match(root, /--car-bg:\s*#000/);
	assert.match(root, /--car-fg:\s*#fff/);
	// No visualiser in car mode.
	assert.doesNotMatch(car, /Visualiser/);
});
