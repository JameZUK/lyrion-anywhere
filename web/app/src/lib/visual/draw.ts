// Drawing the visualiser's styles on a canvas (docs/plans/wave2-design.md §4.1). Pure
// functions of a frame and the canvas size; the component runs them at most 30 times a
// second, and only while Now playing is visible.
import { needlePosition, VU_MAX, VU_MIN, VU_REFERENCE_DBFS } from './vu';
import type { VisualFrame } from './frame';

export type VisualStyle = 'bars' | 'scope' | 'glow' | 'vu';
export const STYLES: readonly VisualStyle[] = ['bars', 'scope', 'glow', 'vu'];
export const STYLE_LABELS: Record<VisualStyle, string> = {
	bars: 'Bars',
	scope: 'Scope',
	glow: 'Glow',
	vu: 'VU meters',
};

/** The next (or previous) style, round the list. */
export function nextStyle(s: VisualStyle, by: 1 | -1 = 1): VisualStyle {
	const i = STYLES.indexOf(s);
	return STYLES[(Math.max(0, i) + by + STYLES.length) % STYLES.length]!;
}

export interface Palette {
	accent: string;
	fg: string;
	muted: string;
	line: string;
	danger: string;
}

/** The parts of CanvasRenderingContext2D used here. */
export type Ctx = Pick<
	CanvasRenderingContext2D,
	'clearRect' | 'fillRect' | 'beginPath' | 'moveTo' | 'lineTo' | 'stroke' | 'arc' | 'fill' | 'fillText' | 'save' | 'restore'
> & {
	fillStyle: CanvasRenderingContext2D['fillStyle'];
	strokeStyle: CanvasRenderingContext2D['strokeStyle'];
	lineWidth: number;
	lineCap: CanvasLineCap;
	font: string;
	textAlign: CanvasTextAlign;
	globalAlpha: number;
};

/** Spectrum bars across the width, rising from the bottom. */
export function drawBars(ctx: Ctx, w: number, h: number, bands: ArrayLike<number>, p: Palette) {
	ctx.clearRect(0, 0, w, h);
	const n = bands.length;
	if (!n || w <= 0 || h <= 0) return;
	const gap = Math.max(1, w / n / 5);
	const bw = (w - gap * (n - 1)) / n;
	ctx.fillStyle = p.accent;
	for (let i = 0; i < n; i++) {
		const v = Math.min(1, Math.max(0, Number(bands[i]) || 0));
		const bh = Math.max(2, v * h * 0.92);
		ctx.fillRect(i * (bw + gap), h - bh, bw, bh);
	}
}

/** The waveform as a line across the middle. */
export function drawScope(ctx: Ctx, w: number, h: number, scope: ArrayLike<number>, p: Palette) {
	ctx.clearRect(0, 0, w, h);
	const n = scope.length;
	if (n < 2 || w <= 0 || h <= 0) return;
	ctx.strokeStyle = p.line;
	ctx.lineWidth = 1;
	ctx.beginPath();
	ctx.moveTo(0, h / 2);
	ctx.lineTo(w, h / 2);
	ctx.stroke();
	ctx.strokeStyle = p.accent;
	ctx.lineWidth = Math.max(2, h / 150);
	ctx.lineCap = 'round';
	ctx.beginPath();
	for (let i = 0; i < n; i++) {
		const v = Math.min(1, Math.max(-1, Number(scope[i]) || 0));
		const x = (i / (n - 1)) * w;
		const y = h / 2 - v * h * 0.45;
		if (i === 0) ctx.moveTo(x, y);
		else ctx.lineTo(x, y);
	}
	ctx.stroke();
}

/** Scale marks on a VU meter (VU), the red zone from 0 up. */
const MARKS = [-20, -10, -7, -5, -3, -1, 0, 1, 2, 3];
/** The needle swings through this angle either side of upright. */
const SWING = (50 * Math.PI) / 180;

function angleAt(pos: number): number {
	return -Math.PI / 2 - SWING + pos * 2 * SWING;
}

/** One needle meter in the box (x, y, w, h). `level` and `peak` in dBFS; 0 VU at `ref` dBFS. */
function meter(ctx: Ctx, x: number, y: number, w: number, h: number, level: number, peak: number, label: string, p: Palette, ref: number) {
	const cx = x + w / 2;
	const cy = y + h * 0.92;
	const r = Math.min(w * 0.46, h * 0.8);
	ctx.lineWidth = Math.max(1, r / 60);
	ctx.lineCap = 'round';
	for (const m of MARKS) {
		const a = angleAt((m - VU_MIN) / (VU_MAX - VU_MIN));
		ctx.strokeStyle = m >= 0 ? p.danger : p.muted;
		ctx.beginPath();
		ctx.moveTo(cx + Math.cos(a) * r * 0.86, cy + Math.sin(a) * r * 0.86);
		ctx.lineTo(cx + Math.cos(a) * r, cy + Math.sin(a) * r);
		ctx.stroke();
	}
	// The arc, red from 0 VU.
	const zero = angleAt((0 - VU_MIN) / (VU_MAX - VU_MIN));
	ctx.strokeStyle = p.muted;
	ctx.beginPath();
	ctx.arc(cx, cy, r * 0.93, angleAt(0), zero);
	ctx.stroke();
	ctx.strokeStyle = p.danger;
	ctx.beginPath();
	ctx.arc(cx, cy, r * 0.93, zero, angleAt(1));
	ctx.stroke();
	// The peak marker, then the needle.
	const pa = angleAt(needlePosition(peak, ref));
	ctx.fillStyle = peak > ref ? p.danger : p.accent;
	ctx.beginPath();
	ctx.arc(cx + Math.cos(pa) * r * 0.78, cy + Math.sin(pa) * r * 0.78, Math.max(2, r / 30), 0, Math.PI * 2);
	ctx.fill();
	const na = angleAt(needlePosition(level, ref));
	ctx.strokeStyle = p.fg;
	ctx.lineWidth = Math.max(1.5, r / 45);
	ctx.beginPath();
	ctx.moveTo(cx, cy);
	ctx.lineTo(cx + Math.cos(na) * r * 0.97, cy + Math.sin(na) * r * 0.97);
	ctx.stroke();
	ctx.fillStyle = p.muted;
	ctx.font = `600 ${Math.max(10, Math.round(r / 7))}px system-ui, sans-serif`;
	ctx.textAlign = 'center';
	ctx.fillText(label, cx, cy - r * 0.35);
}

/** Two needle meters, left and right, stacked when the box is tall and side by side when wide. */
export function drawVu(ctx: Ctx, w: number, h: number, levels: [number, number], peaks: [number, number], p: Palette, ref: number = VU_REFERENCE_DBFS) {
	ctx.clearRect(0, 0, w, h);
	if (w <= 0 || h <= 0) return;
	if (w >= h * 1.3) {
		meter(ctx, 0, 0, w / 2, h, levels[0], peaks[0], 'L', p, ref);
		meter(ctx, w / 2, 0, w / 2, h, levels[1], peaks[1], 'R', p, ref);
	} else {
		meter(ctx, 0, 0, w, h / 2, levels[0], peaks[0], 'L', p, ref);
		meter(ctx, 0, h / 2, w, h / 2, levels[1], peaks[1], 'R', p, ref);
	}
}

/** What the glow style hands the background: the bass, eased so it pulses rather than flickers. */
export function glowLevel(previous: number, frame: Pick<VisualFrame, 'bass'> | null, dtMs: number): number {
	const target = frame && Number.isFinite(frame.bass) ? Math.min(1, Math.max(0, frame.bass)) : 0;
	const prev = Number.isFinite(previous) ? Math.min(1, Math.max(0, previous)) : 0;
	const dt = Number.isFinite(dtMs) ? Math.min(1000, Math.max(0, dtMs)) : 0;
	// Quick to rise, slower to fall.
	const tau = target > prev ? 40 : 180;
	return prev + (target - prev) * (1 - Math.exp(-dt / tau));
}
