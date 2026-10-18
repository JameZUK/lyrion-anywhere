<script lang="ts">
	// Rate a track (Wave 2 §2): five stars (half steps with the arrow keys) or a heart, as
	// the user chose in Settings. Shows the blend of the user's own rating and LMS's shared
	// one; the user's own is in the accent colour, a shared-only rating muted.
	import { ratings } from '$lib/state/ratings';
	import { haptics } from '$lib/haptics/haptics.svelte';

	// `compact`: narrower stars (36 × 44 px each, side by side), to share a row with a title.
	// `toggle`: one button by the title on Now playing (Wave 3.5): a heart, or a star for those
	// who rate in stars, lit when the rating shown is at least three stars: the user's own in
	// the accent colour, or, failing that, the rating LMS holds for the track, muted (so a
	// track rated in LMS before shows as rated here; issue #14). A tap loves the track (five
	// stars) or clears the user's own rating. The full 1–5 rating is in the track's ⋮ menu.
	// `style`: the user's choice from Settings, unless given (the ⋮ menu always shows stars).
	let {
		track,
		size = 22,
		compact = false,
		toggle = false,
		style,
	}: { track: number | null | undefined; size?: number; compact?: boolean; toggle?: boolean; style?: 'stars' | 'heart' } = $props();
	const shape = $derived(style ?? ratings.style);
	const uid = $props.id();

	$effect(() => {
		if (track) void ratings.load([track]);
	});

	const r = $derived(ratings.get(track));
	const shown = $derived(r?.rating ?? 0);
	const own = $derived(r?.mine != null);
	let failed = $state(false);

	async function set(v: number) {
		if (!track) return;
		failed = !(await ratings.rate(track, Math.max(0, Math.min(100, v))));
		if (!failed) haptics.tick('rating');
	}

	/** Clicking the star that is the user's rating clears it. */
	function star(n: number) {
		void set(r?.mine === n * 20 ? 0 : n * 20);
	}

	function keys(e: KeyboardEvent) {
		const now = r?.mine ?? 0;
		const step: Record<string, number> = { ArrowRight: 10, ArrowUp: 10, ArrowLeft: -10, ArrowDown: -10 };
		if (e.key in step) {
			e.preventDefault();
			void set(now + step[e.key]!);
		} else if (e.key === 'Home' || e.key === 'Delete' || e.key === 'Backspace') {
			e.preventDefault();
			void set(0);
		} else if (e.key === 'End') {
			e.preventDefault();
			void set(100);
		}
	}

	const STAR = 'M12 3.2l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17.2l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z';
	const HEART = 'M12 20.3s-7.8-4.7-7.8-10.2A4.4 4.4 0 0 1 12 7.3a4.4 4.4 0 0 1 7.8 2.8c0 5.5-7.8 10.2-7.8 10.2z';
	const fill = (n: number) => Math.max(0, Math.min(1, (shown - (n - 1) * 20) / 20));
</script>

{#if track}
	{#if toggle}
		{@const loved = (r?.mine ?? 0) >= 60}
		{@const on = shown >= 60}
		<button
			class="heart"
			class:on
			class:shared={on && !loved}
			aria-pressed={loved}
			aria-label={on && !loved ? `Rated ${shown / 20} of 5 on the server. Love this track` : 'Love this track'}
			title={loved ? 'Loved. Tap to clear your rating' : on ? `Rated ${shown / 20} of 5 on the server. Tap to love it` : 'Love this track'}
			onclick={() => void set(loved ? 0 : 100)}
			data-rating-toggle={shape}
		>
			<svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><path d={shape === 'heart' ? HEART : STAR} /></svg>
		</button>
	{:else if shape === 'heart'}
		{@const loved = (r?.mine ?? 0) >= 60}
		{@const on = shown >= 60}
		<button
			class="heart"
			class:on
			class:shared={on && !loved}
			aria-pressed={loved}
			aria-label={on && !loved ? `Rated ${shown / 20} of 5 on the server. Love this track` : 'Love this track'}
			title={loved ? 'Loved. Tap to clear your rating' : on ? `Rated ${shown / 20} of 5 on the server. Tap to love it` : 'Love this track'}
			onclick={() => void set(loved ? 0 : 100)}
			data-rating-heart
		>
			<svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><path d={HEART} /></svg>
		</button>
	{:else}
		<div
			class="stars"
			class:own
			class:compact
			role="slider"
			tabindex="0"
			aria-label="Your rating"
			aria-valuemin="0"
			aria-valuemax="100"
			aria-valuenow={r?.mine ?? 0}
			aria-valuetext={r?.mine != null ? `${r.mine / 20} of 5 stars` : shown ? `Not rated by you; ${shown / 20} of 5 on the server` : 'Not rated'}
			onkeydown={keys}
			data-rating-stars
		>
			{#each [1, 2, 3, 4, 5] as n (n)}
				<button tabindex="-1" aria-hidden="true" class="star" onclick={() => star(n)} data-star={n}>
					<svg width={size} height={size} viewBox="0 0 24 24">
						<defs>
							<clipPath id="{uid}-half-{n}"><rect x="0" y="0" width={24 * fill(n)} height="24" /></clipPath>
						</defs>
						<path d={STAR} class="outline" />
						<path d={STAR} class="fill" clip-path="url(#{uid}-half-{n})" />
					</svg>
				</button>
			{/each}
		</div>
	{/if}
	{#if failed}<span class="muted small" role="alert">Couldn’t save the rating</span>{/if}
{/if}

<style>
	.stars {
		display: inline-flex;
		gap: 0;
		border-radius: var(--radius-sm);
		vertical-align: middle;
	}
	.stars:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}
	/* Each star (and the heart) is a 44 × 44 px target, however small it is drawn. */
	.star,
	.heart {
		display: inline-grid;
		place-items: center;
		min-width: 44px;
		min-height: 44px;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		color: var(--fg-muted);
		line-height: 0;
	}
	.compact .star {
		min-width: 36px;
	}
	.outline {
		fill: none;
		stroke: currentColor;
		stroke-width: 1.5;
	}
	.fill {
		fill: var(--fg-muted);
	}
	.own .fill {
		fill: var(--accent);
	}
	.heart path {
		fill: none;
		stroke: currentColor;
		stroke-width: 1.6;
	}
	.heart.on path {
		fill: var(--accent);
		stroke: var(--accent);
	}
	/* Lit only by the rating LMS holds, not the user's own: muted, as the full control's stars. */
	.heart.on.shared path {
		fill: var(--fg-muted);
		stroke: var(--fg-muted);
	}
	.heart:focus-visible {
		outline: 2px solid var(--accent);
		border-radius: 50%;
	}
</style>
