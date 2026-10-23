<script lang="ts">
	// "Your year in sound" (docs/plans/wave3-design.md §1): full-screen cards, one at a time.
	// A carousel with Previous and Next buttons, arrow keys and swipes; nothing moves on its
	// own. Escape or Close ends it and focus goes back to what opened it. Share draws the
	// current card on this device and hands the image to the share sheet (or saves it).
	import { me } from '#lib/state/me';
	import type { ListeningStats } from '#lib/api/me';
	import { deviceTz, storyDaysToGo, yearPeriod } from '#lib/stats/period';
	import { loadNames } from '#lib/stats/names';
	import { storyCards, type StoryCard } from '#lib/stats/story';
	import { shareCard, type ShareDeps } from '#lib/stats/share';
	import Icon from './Icon.svelte';

	let { year, onclose, share = {} }: { year: number; onclose: () => void; share?: ShareDeps } = $props();

	let dialog = $state<HTMLDialogElement>();
	let cardEl = $state<HTMLElement>();
	let nextBtn = $state<HTMLButtonElement>();
	let stats = $state<ListeningStats | null>(null);
	let cards = $state<StoryCard[]>([]);
	let error = $state('');
	let index = $state(0);
	let message = $state('');
	let sharing = $state(false);
	const id = `story-${Math.random().toString(36).slice(2, 9)}`;
	const opener = typeof document !== 'undefined' ? (document.activeElement as HTMLElement | null) : null;
	let closed = false;

	async function load() {
		error = '';
		try {
			const s = await me.stats(yearPeriod(year), deviceTz());
			const names = await loadNames(s.top_albums.slice(0, 1).map((a) => a.id), s.top_tracks.slice(0, 1).map((t) => t.id));
			stats = s;
			cards = storyCards(year, s, names);
			index = 0;
		} catch (e) {
			error = (e as Error).message || 'Your year in sound couldn’t be loaded.';
		}
	}
	void load();

	$effect(() => {
		const d = dialog;
		if (d && !d.open) d.showModal?.();
	});

	const ready = $derived(!!stats && stats.enabled && storyDaysToGo(stats.totals.days) === 0 && cards.length > 0);
	const card = $derived(cards[index]);

	// Focus the first card once it's there, so a screen reader starts reading it.
	let focusedOnce = false;
	$effect(() => {
		if (ready && cardEl && !focusedOnce) {
			focusedOnce = true;
			cardEl.focus();
		}
	});

	function go(to: number) {
		if (!ready) return;
		const n = Math.max(0, Math.min(cards.length - 1, to));
		if (n === index) return;
		index = n;
		message = '';
		// Previous just became disabled: keep focus somewhere useful.
		if (n === 0 && document.activeElement instanceof HTMLButtonElement && document.activeElement.dataset.storyPrev !== undefined) nextBtn?.focus();
	}

	function close() {
		if (closed) return;
		closed = true;
		if (dialog?.open) dialog.close();
		if (opener?.isConnected) opener.focus();
		onclose();
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		} else if (e.key === 'ArrowRight') {
			e.preventDefault();
			go(index + 1);
		} else if (e.key === 'ArrowLeft') {
			e.preventDefault();
			go(index - 1);
		}
	}

	// Swipes: a mostly sideways drag of 48 px or more turns the card.
	let start: { x: number; y: number; id: number } | null = null;
	function down(e: PointerEvent) {
		start = { x: e.clientX, y: e.clientY, id: e.pointerId };
	}
	function up(e: PointerEvent) {
		if (!start || start.id !== e.pointerId) return;
		const dx = e.clientX - start.x;
		const dy = e.clientY - start.y;
		start = null;
		if (Math.abs(dx) < 48 || Math.abs(dx) < Math.abs(dy)) return;
		go(index + (dx < 0 ? 1 : -1));
	}

	async function doShare() {
		if (!card || sharing) return;
		sharing = true;
		message = '';
		try {
			const r = await shareCard(card, year, share);
			message = r === 'saved' ? 'Image saved. It was made on this device; nothing was uploaded.' : r === 'shared' ? 'Shared.' : '';
		} catch (e) {
			message = (e as Error).message || 'The image couldn’t be made.';
		} finally {
			sharing = false;
		}
	}
</script>

<dialog
	bind:this={dialog}
	class="story"
	aria-labelledby={`${id}-t`}
	aria-modal="true"
	{onkeydown}
	oncancel={(e) => {
		e.preventDefault();
		close();
	}}
	onclose={close}
	data-year-story
>
	<div class="frame">
		<header>
			<h2 id={`${id}-t`}>Your year in sound {year}</h2>
			<button class="icon-btn close" aria-label="Close" onclick={close} data-story-close><Icon name="close" size={22} /></button>
		</header>
		{#if error}
			<div class="note">
				<p>{error}</p>
				<button class="btn" onclick={load}>Try again</button>
			</div>
		{:else if !stats}
			<p class="note" role="status">Loading…</p>
		{:else if !stats.enabled}
			<div class="note">
				<p>Listening history is switched off, so there’s no year to tell.</p>
				<a class="btn" href="#/settings" onclick={close}>Listening history settings</a>
			</div>
		{:else if !ready}
			{@const left = storyDaysToGo(stats.totals.days)}
			<p class="note" role="status">
				Your year in sound needs a week of listening in {year}. {left} more {left === 1 ? 'day' : 'days'} with music to go.
			</p>
		{:else if card}
			<div class="stage" role="region" aria-roledescription="carousel" aria-label={`Your year in sound ${year}`} aria-live="polite" onpointerdown={down} onpointerup={up} onpointercancel={() => (start = null)}>
				{#key index}
					<section
						class="card"
						role="group"
						aria-roledescription="slide"
						aria-label={`${index + 1} of ${cards.length}`}
						tabindex="-1"
						bind:this={cardEl}
						data-story-card={card.key}
					>
						<p class="kicker">{card.kicker}</p>
						<p class="headline">{card.headline}</p>
						<p class="detail">{card.detail}</p>
						{#if card.list}
							<ol>{#each card.list as item, i (i)}<li>{item}</li>{/each}</ol>
						{/if}
					</section>
				{/key}
			</div>
			<div class="dots" aria-hidden="true">
				{#each cards as c, i (c.key)}<span class:on={i === index}></span>{/each}
			</div>
			<div class="controls">
				<button class="btn" onclick={() => go(index - 1)} disabled={index === 0} data-story-prev><Icon name="prev" size={18} />Previous</button>
				<button class="btn" onclick={doShare} disabled={sharing}>Share</button>
				{#if index < cards.length - 1}
					<button class="btn primary" bind:this={nextBtn} onclick={() => go(index + 1)} data-story-next>Next<Icon name="next" size={18} /></button>
				{:else}
					<button class="btn primary" bind:this={nextBtn} onclick={close} data-story-next>Done</button>
				{/if}
			</div>
		{/if}
		<p class="status" role="status">{message}</p>
	</div>
</dialog>

<style>
	.story {
		border: 0;
		padding: 0;
		margin: 0;
		width: 100vw;
		max-width: 100vw;
		height: var(--app-h, 100vh);
		max-height: none;
		background: var(--bg);
		color: var(--fg);
	}
	.story::backdrop {
		background: rgb(0 0 0 / 60%);
	}
	.frame {
		display: flex;
		flex-direction: column;
		height: 100%;
		max-width: 640px;
		margin: 0 auto;
		padding: max(1rem, env(safe-area-inset-top)) 1rem max(1rem, env(safe-area-inset-bottom));
		box-sizing: border-box;
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
	}
	h2 {
		margin: 0;
		font-size: 1rem;
		color: var(--fg-muted);
	}
	.close {
		width: 44px;
		height: 44px;
		color: var(--fg);
	}
	.note {
		margin: auto 0;
		text-align: center;
	}
	.stage {
		flex: 1;
		display: flex;
		align-items: center;
		min-height: 0;
		touch-action: pan-y;
		user-select: none;
	}
	.card {
		width: 100%;
		padding: 2rem 1.5rem;
		border-radius: var(--radius);
		background: linear-gradient(160deg, var(--accent-soft), var(--bg-raised));
		border: 1px solid var(--line);
		animation: arrive 0.28s ease-out;
		outline-offset: 4px;
	}
	.kicker {
		margin: 0 0 1rem;
		color: var(--accent);
		font-weight: 600;
	}
	.headline {
		margin: 0 0 0.75rem;
		font-size: clamp(1.8rem, 7vw, 3rem);
		font-weight: 700;
		line-height: 1.1;
		overflow-wrap: anywhere;
	}
	.detail {
		margin: 0;
		color: var(--fg-muted);
	}
	ol {
		margin: 1rem 0 0;
		padding-left: 1.4rem;
		font-size: 1.15rem;
		font-weight: 600;
	}
	.dots {
		display: flex;
		justify-content: center;
		gap: 0.4rem;
		margin: 1rem 0;
	}
	.dots span {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--line);
	}
	.dots span.on {
		background: var(--accent);
	}
	.controls {
		display: flex;
		justify-content: space-between;
		gap: 0.5rem;
	}
	.controls .btn {
		min-height: 44px;
		min-width: 44px;
	}
	.status {
		min-height: 1.4em;
		margin: 0.5rem 0 0;
		text-align: center;
		font-size: 0.9rem;
	}
	@keyframes arrive {
		from {
			opacity: 0;
			transform: translateX(24px);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.card {
			animation: none;
		}
	}
</style>
