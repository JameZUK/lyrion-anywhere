<script lang="ts">
	// The visualiser (docs/plans/wave2-design.md §4.1), where the cover is on Now playing. It
	// is decorative: hidden from screen readers, with its on/off and style controls on the
	// page. While it is mounted and the page is visible it asks this device's player for
	// frames (the worker analyses the audio only then) and draws at most 30 times a second;
	// when the page is hidden it stops both. Glow draws nothing here: it finds the beats in
	// the bass (lib/visual/glow.ts) and hands the page the halo's pulse each frame.
	import { device } from '#lib/player/device.svelte';
	import { parseFrame, silentFrame, type VisualFrame } from '#lib/visual/frame';
	import { drawBars, drawScope, drawVu, glowLevel, nextStyle, type Palette, type VisualStyle } from '#lib/visual/draw';
	import { BeatDetector, GLOW_REST, PulseEnvelope, glowStyle, type GlowStyle } from '#lib/visual/glow';
	import { VuBallistics, VuCalibration, fallBars } from '#lib/visual/vu';

	let {
		style,
		onstyle,
		onglow,
	}: {
		style: VisualStyle;
		/** A swipe across it asks for the next (or previous) style. */
		onstyle?: (s: VisualStyle) => void;
		/** Glow: the halo's size, opacity and the cover's scale, each drawn frame (at rest when it stops). */
		onglow?: (g: GlowStyle) => void;
	} = $props();

	/** No frame for this long (paused, stopped, a gap): fall back to silence. */
	const STALE_MS = 400;
	const FRAME_MS = 1000 / 30;
	const SWIPE_PX = 50;

	let box = $state<HTMLDivElement>();
	let canvas = $state<HTMLCanvasElement>();

	$effect(() => {
		let frame: VisualFrame = silentFrame();
		let frameAt = 0;
		const bars = new Float32Array(frame.bands.length);
		const vu = [new VuBallistics(), new VuBallistics()];
		const calibration = new VuCalibration();
		let glow = 0;
		const beats = new BeatDetector();
		const pulse = new PulseEnvelope();
		let palette: Palette | null = null;
		let paletteAt = -Infinity;
		let ctx: CanvasRenderingContext2D | null | undefined;
		let raf = 0;
		let last = 0;
		let unwatch: (() => void) | null = null;
		const raff = typeof requestAnimationFrame === 'function' ? requestAnimationFrame : (cb: FrameRequestCallback) => setTimeout(() => cb(performance.now()), FRAME_MS) as unknown as number;
		const caf = typeof cancelAnimationFrame === 'function' ? cancelAnimationFrame : (id: number) => clearTimeout(id);

		function readPalette(): Palette {
			const cs = box ? getComputedStyle(box) : null;
			const v = (name: string, fallback: string) => cs?.getPropertyValue(name).trim() || fallback;
			return { accent: v('--accent', '#3b82f6'), fg: v('--fg', '#ffffff'), muted: v('--fg-muted', '#999999'), line: v('--line', '#444444'), danger: v('--danger', '#e5484d') };
		}

		function draw(t: number, dt: number) {
			if (t - frameAt > STALE_MS) frame = silentFrame();
			const s = style;
			if (s === 'glow') {
				glow = glowLevel(glow, frame, dt);
				onglow?.(glowStyle(pulse.value(performance.now()), glow));
				return;
			}
			const c = canvas;
			if (!c || !box) return;
			const dpr = Math.min(2, globalThis.devicePixelRatio || 1);
			const w = Math.round(box.clientWidth * dpr);
			const h = Math.round(box.clientHeight * dpr);
			if (w <= 0 || h <= 0) return;
			if (c.width !== w || c.height !== h) {
				c.width = w;
				c.height = h;
			}
			if (ctx === undefined) {
				try {
					ctx = c.getContext('2d');
				} catch {
					ctx = null;
				}
			}
			if (!ctx) return;
			// The theme can change while it runs: read its colours now and then.
			if (!palette || t - paletteAt > 1000) {
				palette = readPalette();
				paletteAt = t;
			}
			if (s === 'bars') drawBars(ctx, w, h, fallBars(bars, frame.bands, dt), palette);
			else if (s === 'scope') drawScope(ctx, w, h, frame.scope, palette);
			else {
				const now = performance.now();
				vu[0]!.step(frame.rmsDb[0], frame.peakDb[0], dt, now);
				vu[1]!.step(frame.rmsDb[1], frame.peakDb[1], dt, now);
				// Only while music is heard (a stale frame is silence, which it ignores).
				const ref = calibration.step(frame.rmsDb, dt);
				drawVu(ctx, w, h, [vu[0]!.level, vu[1]!.level], [vu[0]!.peak, vu[1]!.peak], palette, ref);
			}
		}

		function loop(t: number) {
			raf = raff(loop);
			if (last && t - last < FRAME_MS - 2) return;
			const dt = last ? t - last : 0;
			last = t;
			draw(t, dt);
		}

		function start() {
			if (unwatch) return;
			unwatch = device.watchVisual((f) => {
				const p = parseFrame(f);
				if (p) {
					frame = p;
					frameAt = performance.now();
					// Each new frame once: a beat kicks the halo's pulse.
					if (style === 'glow' && beats.step(p.bands, frameAt)) pulse.kick(frameAt);
				}
			});
			last = 0;
			raf = raff(loop);
		}
		function stop() {
			unwatch?.();
			unwatch = null;
			caf(raf);
			raf = 0;
			beats.reset();
			pulse.reset();
			onglow?.(GLOW_REST);
		}
		const onVisible = () => (document.visibilityState === 'hidden' ? stop() : start());
		onVisible();
		document.addEventListener('visibilitychange', onVisible);
		return () => {
			document.removeEventListener('visibilitychange', onVisible);
			stop();
		};
	});

	// A swipe across the visualiser changes its style (touch, pen or mouse); the style button
	// on the page does the same.
	let swipe: { id: number; x: number; y: number } | null = null;
	function down(e: PointerEvent) {
		swipe = { id: e.pointerId, x: e.clientX, y: e.clientY };
	}
	function up(e: PointerEvent) {
		const s = swipe;
		swipe = null;
		if (!s || s.id !== e.pointerId) return;
		const dx = e.clientX - s.x;
		const dy = e.clientY - s.y;
		if (Math.abs(dx) >= SWIPE_PX && Math.abs(dx) > Math.abs(dy) * 1.5) onstyle?.(nextStyle(style, dx < 0 ? 1 : -1));
	}
</script>

<div
	class="vis"
	class:glow={style === 'glow'}
	bind:this={box}
	aria-hidden="true"
	data-visualiser={style}
	onpointerdown={down}
	onpointerup={up}
	onpointercancel={() => (swipe = null)}
>
	{#if style !== 'glow'}<canvas bind:this={canvas}></canvas>{/if}
</div>

<style>
	.vis {
		position: relative;
		width: 100%;
		aspect-ratio: 1;
		border-radius: var(--radius);
		background: color-mix(in srgb, var(--bg-sunken) 55%, transparent);
		touch-action: pan-y pinch-zoom;
		user-select: none;
	}
	.vis.glow {
		position: absolute;
		inset: 0;
		/* Above the cover, so a swipe across it still changes the style. */
		z-index: 2;
		aspect-ratio: auto;
		background: none;
	}
	canvas {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		border-radius: inherit;
	}
</style>
