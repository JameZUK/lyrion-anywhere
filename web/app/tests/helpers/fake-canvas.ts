// A stand-in for a browser canvas (jsdom has none): records the text drawn, makes a tiny PNG blob.
import type { CanvasLike, Ctx2D } from '#lib/stats/share';

export function fakeCanvas() {
	const texts: string[] = [];
	const rects: number[][] = [];
	const ctx: Ctx2D = {
		fillStyle: '',
		font: '',
		textAlign: 'left',
		textBaseline: 'top',
		fillRect: (...r: number[]) => void rects.push(r),
		fillText: (t: string) => void texts.push(t),
		measureText: (t: string) => ({ width: t.length * 30 }),
	};
	const canvas: CanvasLike & { texts: string[]; rects: number[][]; blobs: number } = {
		width: 0,
		height: 0,
		texts,
		rects,
		blobs: 0,
		getContext: () => ctx,
		toBlob(cb, type) {
			canvas.blobs++;
			cb(new Blob(['PNG'], { type }));
		},
	};
	return canvas;
}
