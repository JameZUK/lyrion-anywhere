<script lang="ts">
	// The card at the end of Home: this month's listening, opening Your listening. From
	// 1 December to 31 January it also offers "Your year in sound" for the year. Hidden while
	// listening history is off or there is nothing to show.
	import { me } from '#lib/state/me';
	import type { ListeningStats } from '#lib/api/me';
	import { deviceTz, duration, monthPeriod, number, storyDaysToGo, storyYear, yearPeriod } from '#lib/stats/period';
	import Icon from './Icon.svelte';
	import YearStory from './YearStory.svelte';

	const now = new Date();
	const offered = storyYear(now);
	let month = $state<ListeningStats | null>(null);
	let year = $state<ListeningStats | null>(null);
	let story = $state(false);

	async function load() {
		const tz = deviceTz(now);
		try {
			const [m, y] = await Promise.all([me.stats(monthPeriod(now), tz), offered === null ? Promise.resolve(null) : me.stats(yearPeriod(offered), tz).catch(() => null)]);
			month = m;
			year = y;
		} catch {
			month = null; // no card rather than an error on Home
		}
	}
	void load();

	const show = $derived(!!month && month.enabled && month.totals.plays > 0);
	const showStory = $derived(!!year && year.enabled && storyDaysToGo(year.totals.days) === 0);
</script>

{#if show || showStory}
	<div class="stats-card" data-stats-card>
		{#if show && month}
			<a class="summary" href="#/stats">
				<Icon name="signal" size={20} />
				<span>This month: {duration(month.totals.seconds)} · {number(month.totals.plays)} {month.totals.plays === 1 ? 'play' : 'plays'}</span>
				<span class="chev" aria-hidden="true">›</span>
			</a>
		{/if}
		{#if showStory && offered !== null}
			<button class="btn primary" onclick={() => (story = true)} data-story-offer>Your year in sound {offered}</button>
		{/if}
	</div>
	{#if story && offered !== null}
		<YearStory year={offered} onclose={() => (story = false)} />
	{/if}
{/if}

<style>
	.stats-card {
		display: grid;
		gap: 0.6rem;
		max-width: 560px;
		margin-top: 1rem;
		padding: 0.8rem 0.9rem;
		border-radius: var(--radius);
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	.summary {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 44px;
		font-weight: 600;
	}
	.chev {
		margin-left: auto;
		color: var(--fg-muted);
		font-size: 1.4rem;
	}
	.btn {
		min-height: 44px;
		justify-self: start;
	}
</style>
