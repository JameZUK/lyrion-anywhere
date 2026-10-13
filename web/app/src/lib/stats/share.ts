// Sharing a "Your year in sound" card: the card is drawn to a canvas on this device and the
// PNG handed to the platform's share sheet (navigator.share with a file), or, where that
// isn't offered, saved as a download. Nothing is uploaded anywhere.
import type { StoryCard } from './story';

/** The parts of a 2D context the drawing uses (so tests can pass a fake). */
export interface Ctx2D {
	fillStyle: string | CanvasGradient | CanvasPattern;
	font: string;
	textAlign: CanvasTextAlign;
	textBaseline: CanvasTextBaseline;
	fillRect(x: number, y: number, w: number, h: number): void;
	fillText(text: string, x: number, y: number): void;
	measureText(text: string): { width: number };
}

export interface CanvasLike {
	width: number;
	height: number;
	getContext(kind: '2d'): Ctx2D | null;
	toBlob(cb: (b: Blob | null) => void, type?: string): void;
}

export interface ShareDeps {
	makeCanvas?: () => CanvasLike;
	nav?: Pick<Navigator, 'share' | 'canShare'> | null;
	doc?: Document;
	url?: { createObjectURL(b: Blob): string; revokeObjectURL(u: string): void };
}

export const IMAGE = { width: 1080, height: 1350 } as const;
const PAD = 96;
const FONT = "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif";
const COLOURS = { bg: '#1a120d', fg: '#f7f2ec', muted: '#c9bfb4', accent: '#fb923c' };

/** Words broken into lines no wider than `max` (a single over-long word gets a line of its own). */
export function wrap(ctx: Pick<Ctx2D, 'measureText'>, text: string, max: number, maxLines = 6): string[] {
	const lines: string[] = [];
	let cur = '';
	for (const word of text.split(/\s+/).filter(Boolean)) {
		const next = cur ? `${cur} ${word}` : word;
		if (cur && ctx.measureText(next).width > max) {
			lines.push(cur);
			cur = word;
		} else cur = next;
	}
	if (cur) lines.push(cur);
	if (lines.length > maxLines) {
		const kept = lines.slice(0, maxLines);
		kept[maxLines - 1] = `${kept[maxLines - 1]}…`;
		return kept;
	}
	return lines;
}

/** Draw `card` onto a 1080 × 1350 context, as text only (fillText: never markup). */
export function drawCard(ctx: Ctx2D, card: StoryCard, footer: string): void {
	const { width: w, height: h } = IMAGE;
	const max = w - PAD * 2;
	ctx.fillStyle = COLOURS.bg;
	ctx.fillRect(0, 0, w, h);
	ctx.fillStyle = COLOURS.accent;
	ctx.fillRect(PAD, PAD, 120, 12);
	ctx.textAlign = 'left';
	ctx.textBaseline = 'top';
	let y = PAD + 72;
	ctx.font = `600 44px ${FONT}`;
	ctx.fillStyle = COLOURS.accent;
	for (const line of wrap(ctx, card.kicker, max, 2)) {
		ctx.fillText(line, PAD, y);
		y += 58;
	}
	y += 40;
	ctx.font = `700 96px ${FONT}`;
	ctx.fillStyle = COLOURS.fg;
	for (const line of wrap(ctx, card.headline, max, 4)) {
		ctx.fillText(line, PAD, y);
		y += 112;
	}
	y += 32;
	ctx.font = `400 44px ${FONT}`;
	ctx.fillStyle = COLOURS.muted;
	for (const line of wrap(ctx, card.detail, max, 4)) {
		ctx.fillText(line, PAD, y);
		y += 60;
	}
	if (card.list?.length) {
		y += 32;
		ctx.font = `600 52px ${FONT}`;
		ctx.fillStyle = COLOURS.fg;
		card.list.slice(0, 5).forEach((item, i) => {
			const [line] = wrap(ctx, `${i + 1}. ${item}`, max, 1);
			ctx.fillText(line ?? '', PAD, y);
			y += 76;
		});
	}
	ctx.font = `500 36px ${FONT}`;
	ctx.fillStyle = COLOURS.muted;
	ctx.fillText(footer, PAD, h - PAD - 36);
}

/** The card as a PNG, made on this device. */
export function cardImage(card: StoryCard, footer: string, makeCanvas: () => CanvasLike = defaultCanvas): Promise<Blob> {
	const canvas = makeCanvas();
	canvas.width = IMAGE.width;
	canvas.height = IMAGE.height;
	const ctx = canvas.getContext('2d');
	if (!ctx) return Promise.reject(new Error('This browser can’t make images.'));
	drawCard(ctx, card, footer);
	return new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('The image couldn’t be made.'))), 'image/png'));
}

function defaultCanvas(): CanvasLike {
	return document.createElement('canvas') as unknown as CanvasLike;
}

/** A safe file name for the image: "your-year-2026-top-artist.png". */
export function imageName(year: number, card: StoryCard): string {
	const key = card.key.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 20) || 'card';
	return `your-year-${Math.trunc(year)}-${key}.png`;
}

/**
 * Share the card: through the share sheet with the image where the browser can share files,
 * else saved as a PNG. 'cancelled' when the user closed the share sheet.
 */
export async function shareCard(card: StoryCard, year: number, deps: ShareDeps = {}): Promise<'shared' | 'saved' | 'cancelled'> {
	const blob = await cardImage(card, `Your year in sound ${year} · Lyrion Anywhere`, deps.makeCanvas);
	const name = imageName(year, card);
	const nav = deps.nav === undefined ? (typeof navigator === 'undefined' ? null : navigator) : deps.nav;
	if (nav && typeof nav.share === 'function' && typeof nav.canShare === 'function' && typeof File === 'function') {
		const file = new File([blob], name, { type: 'image/png' });
		if (nav.canShare({ files: [file] })) {
			try {
				await nav.share({ files: [file], title: `Your year in sound ${year}` });
				return 'shared';
			} catch (e) {
				if ((e as Error)?.name === 'AbortError') return 'cancelled';
				// Refused for another reason (no user gesture left, say): save it instead.
			}
		}
	}
	const doc = deps.doc ?? document;
	const url = deps.url ?? URL;
	const href = url.createObjectURL(blob);
	const a = doc.createElement('a');
	a.href = href;
	a.download = name;
	a.rel = 'noopener';
	a.style.display = 'none';
	doc.body.append(a);
	a.click();
	a.remove();
	setTimeout(() => url.revokeObjectURL(href), 10_000);
	return 'saved';
}
