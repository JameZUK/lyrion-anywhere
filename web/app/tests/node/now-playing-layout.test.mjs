// Now playing on phones (node --test): jsdom has no layout, so the CSS that keeps the page from
// scrolling, shrinks the cover to fit and keeps every control on screen at 360 × 640 is checked
// here, from the styles themselves.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (p) => readFileSync(new URL(`../../src/${p}`, import.meta.url), 'utf8');
const style = (src) => src.slice(src.indexOf('<style>'), src.indexOf('</style>'));
/** The declarations of the first rule for `selector` (a selector list ending in ` {`), after `from`. */
function rule(css, selector, from = 0) {
	const at = css.indexOf(`${selector} {`, from);
	assert.ok(at >= 0, `no rule for ${selector}`);
	const open = css.indexOf('{', at);
	return css.slice(open + 1, css.indexOf('}', open));
}
const decl = (d, prop) => d.match(new RegExp(`(?:^|[;\\s])${prop}:\\s*([^;]+);`))?.[1]?.trim();
const px = (d, prop) => Number(decl(d, prop)?.match(/^(-?\d+)px$/)?.[1] ?? NaN);

const phone = style(read('lib/components/NowPlayingPhone.svelte'));
const pager = style(read('lib/components/NowPlayingPager.svelte'));

test('the column never scrolls the page: it fills the space it is given and hides overflow', () => {
	const np = rule(phone, '.np-phone');
	assert.equal(decl(np, 'flex'), '1 1 0');
	assert.equal(decl(np, 'min-height'), '0');
	assert.equal(decl(np, 'overflow'), 'hidden');
	assert.equal(decl(np, 'flex-direction'), 'column');
	// Only a phone on its side (under 460 px tall) lets the column scroll, inside itself.
	const sideways = phone.indexOf('@media (max-height: 460px)');
	assert.ok(sideways > 0);
	const outside = phone.slice(0, sideways);
	assert.equal((outside.match(/\.np-phone \{/g) ?? []).length, 1, 'one rule for the column outside that block');
	for (const sel of ['.np-phone', '.hero', '.top', '.meta', '.seek', '.transport', '.next', '.actions']) {
		assert.doesNotMatch(rule(outside, sel), /overflow(-y)?:\s*(auto|scroll|visible)|height:\s*auto/, sel);
	}
});

test('the layout makes the page exactly the visible height (100dvh) on phones and narrow windows', () => {
	const layout = read('routes/+layout.svelte');
	const narrow = layout.indexOf('@media (max-width: 840px)');
	assert.ok(narrow >= 0, 'a block for windows up to 840 px');
	const content = rule(layout, '.content.on-np', narrow);
	assert.equal(decl(content, 'height'), 'var(--app-h)');
	assert.equal(decl(content, 'overflow'), 'hidden');
	assert.equal(decl(content, 'display'), 'flex');
	assert.equal(decl(rule(layout, '.barwrap.on-np', narrow), 'display'), 'none');
	assert.match(read('app.css'), /--app-h:\s*100dvh/);
	// The page picks the phone layout below 841 px.
	assert.match(read('routes/+page.svelte'), /mq\('\(min-width: 841px\)'\)/);
	assert.match(read('routes/+page.svelte'), /\{#if !wide\}\s*<NowPlayingPhone/);
});

test('only the hero grows or shrinks; the cover is square and as big as fits', () => {
	const hero = rule(phone, '.hero');
	assert.equal(decl(hero, 'flex'), '1 1 0');
	assert.equal(decl(hero, 'min-height'), '0');
	const p = rule(pager, '.pager');
	assert.equal(decl(p, 'min-height'), '0');
	assert.equal(decl(p, 'overflow'), 'hidden');
	assert.equal(decl(rule(phone, '.stage'), 'container-type'), 'size');
	const art = rule(phone, '.art-box');
	assert.equal(decl(art, 'width'), 'min(100cqw, 100cqh)');
	assert.equal(decl(art, 'aspect-ratio'), '1');
	// Lyrics and track info scroll inside their page only.
	const scroll = rule(phone, '.scroll');
	assert.equal(decl(scroll, 'overflow-y'), 'auto');
	assert.equal(decl(scroll, 'min-height'), '0');
	assert.equal(decl(scroll, 'overscroll-behavior'), 'contain');
	for (const fixed of ['.top', '.meta', '.seek', '.transport', '.next', '.actions']) {
		assert.equal(decl(rule(phone, fixed), 'flex'), 'none', `${fixed} keeps its height`);
	}
});

test('at 360 × 640 every control fits with room for a cover of at least 120 px', () => {
	const W = 360;
	const H = 640;
	const TABS = 52; // the tab bar (routes/+layout.svelte, --tabs-h without a safe area)
	const np = rule(phone, '.np-phone');
	const pad = decl(np, 'padding').match(/^max\((\d+)px, env\(safe-area-inset-top\)\)\s+(\d+)px\s+(\d+)px$/);
	assert.ok(pad, 'padding: top, sides, bottom');
	const [top, side, bottom] = [Number(pad[1]), Number(pad[2]), Number(pad[3])];
	const gap = px(np, 'gap');
	const height = (sel, prop) => {
		const v = px(rule(phone, sel), prop);
		assert.ok(Number.isFinite(v), `${sel} ${prop}`);
		return v;
	};
	const rows = [
		height('.top', 'height'),
		height('.meta', 'min-height'),
		height('.seek', 'min-height'),
		height('.transport', 'height'),
		height('.next', 'max-height'),
		height('.actions', 'min-height'),
	];
	const dots = px(rule(pager, '.dots'), 'height');
	const pagerGap = px(rule(pager, '.pager-wrap'), 'gap');
	// Seven rows in the column (the hero among them): six gaps.
	const fixed = top + bottom + rows.reduce((a, b) => a + b, 0) + 6 * gap + dots + pagerGap;
	const heroLeft = H - TABS - fixed;
	const cover = Math.min(W - 2 * side, heroLeft);
	assert.ok(cover >= 120, `cover ${cover} px (fixed rows ${fixed} px)`);
	// The controls inside the rows fit their rows.
	assert.ok(px(rule(phone, '.transport .play'), 'height') <= height('.transport', 'height'));
	assert.ok(px(rule(phone, '.top .icon-btn'), 'height') <= height('.top', 'height'));
	assert.ok(px(rule(phone, '.title-row'), 'height') + px(rule(phone, '.sub'), 'height') <= height('.meta', 'min-height'));
	// Shorter screens give up Up next before anything else.
	assert.match(phone, /@media \(max-height: 600px\) \{\s*\.next \{\s*display: none;/);
});

test('phone controls are 44 px targets (or more)', () => {
	const css = read('app.css');
	const act = rule(css, '.np-act');
	assert.ok(px(act, 'min-height') >= 44 && px(act, 'min-width') >= 44, act);
	assert.ok(px(rule(phone, '.top .icon-btn'), 'width') >= 44);
	assert.ok(px(rule(phone, '.transport .icon-btn'), 'width') >= 44);
	assert.ok(px(rule(phone, '.row'), 'min-height') >= 44);
	// The small badge and dots reach 44 px tall through their ::after.
	const badge = px(rule(phone, '.badge'), 'height');
	const reach = Number(decl(rule(phone, '.badge::after'), 'inset').match(/^-(\d+)px/)[1]);
	assert.ok(badge + 2 * reach >= 44, `badge ${badge} + 2 × ${reach}`);
	const dot = rule(pager, '.dots button');
	assert.ok(px(dot, 'width') >= 44);
	const dotReach = Number(decl(rule(pager, '.dots button::after'), 'inset').match(/^-(\d+)px/)[1]);
	assert.ok(px(dot, 'height') + 2 * dotReach >= 44);
	// The volume slider (in Play on…) is a 44 px tall target.
	assert.ok(px(rule(style(read('lib/components/VolumeSlider.svelte')), 'input'), 'height') >= 44);
	// The one rating toggle beside the title is a 44 px target (the full stars are in ⋮).
	const rating = style(read('lib/components/RatingControl.svelte'));
	const toggle = rule(rating, '.star,\n\t.heart');
	assert.ok(px(toggle, 'min-width') >= 44 && px(toggle, 'min-height') >= 44);
	assert.match(read('lib/components/NowPlayingPhone.svelte'), /<RatingControl track=\{libraryId\} size=\{22\} toggle \/>/);
});

test('reduced motion: the pager does not slide', () => {
	assert.match(pager, /@media \(prefers-reduced-motion: reduce\) \{\s*\.track \{\s*transition: none;/);
	assert.equal(decl(rule(pager, '.track.still'), 'transition'), 'none');
});

test('Wave 3.5: the player bar is hidden under Now playing at every width (no second transport)', () => {
	const css = style(read('routes/+layout.svelte'));
	const firstMedia = css.indexOf('@media');
	assert.ok(css.indexOf('.barwrap.on-np {') < firstMedia, 'outside any media query');
	assert.equal(decl(rule(css, '.barwrap.on-np'), 'display'), 'none');
	// And the page keeps no room for it.
	assert.doesNotMatch(rule(css, '.content.on-np', css.indexOf('.barwrap.on-np {')), /--bar-h/);
});
