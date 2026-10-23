<script lang="ts">
	// Your listening (docs/plans/wave3-design.md §1): totals, most played, new artists, when
	// you listen, streaks, for a chosen period, from the user's own listening history.
	import { me } from '#lib/state/me';
	import BackButton from '#lib/components/BackButton.svelte';
	import StatsChart from '#lib/components/StatsChart.svelte';
	import YearStory from '#lib/components/YearStory.svelte';
	import type { ListeningStats } from '#lib/api/me';
	import { link } from '#lib/lms/commands';
	import { textOf } from '#lib/lms/format';
	import { loadNames, noNames, type StatsNames } from '#lib/stats/names';
	import * as P from '#lib/stats/period';

	const now = new Date();
	let period = $state(P.monthPeriod(now));
	let retention = $state(396);
	let stats = $state<ListeningStats | null>(null);
	let names = $state<StatsNames>(noNames());
	let error = $state('');
	let loading = $state(false);
	let story = $state<number | null>(null);
	let more = $state({ artists: false, albums: false, tracks: false });
	let seq = 0;

	async function load() {
		const mine = ++seq;
		error = '';
		loading = true;
		more = { artists: false, albums: false, tracks: false };
		try {
			const s = await me.stats(period, P.deviceTz(now));
			if (mine !== seq) return;
			stats = s;
			retention = s.retention_days || retention;
			names = noNames();
			const n = await loadNames(
				s.top_albums.map((a) => a.id),
				s.top_tracks.map((t) => t.id),
			);
			if (mine === seq) names = n;
		} catch (e) {
			if (mine === seq) error = (e as Error).message || 'Your listening couldn’t be loaded.';
		} finally {
			if (mine === seq) loading = false;
		}
	}
	void load();

	const choices = $derived(P.periodChoices(now, retention));
	const words = $derived(P.periodWords(period, now));
	const year = $derived(P.periodYear(period));
	const since = $derived(P.newMeansSince(period));
	const oldest = $derived(P.dayName(new Date(now.getTime() - retention * 86400_000).toISOString().slice(0, 10)));
	const HOUR_NAMES = Array.from({ length: 24 }, (_, h) => P.hourRange(h));
	const HOUR_LABELS = Array.from({ length: 24 }, (_, h) => String(h));

	const albumName = (id: number) => {
		const a = names.albums.get(id);
		return a ? textOf(a.album) || 'An untitled album' : 'An album no longer in the library';
	};
	const trackOf = (id: number) => names.tracks.get(id);
	const trackLink = (id: number) => {
		const t = trackOf(id);
		return t?.album_id !== undefined ? link('album', t.album_id) : null;
	};
	const amount = (p: number, s: number) => `${P.plays(p)} · ${P.duration(s)}`;
</script>

<section class="page stats">
	<BackButton parent="/home" />
	<h1>Your listening</h1>
	<label class="field period">
		<span>Period</span>
		<select bind:value={period} onchange={load} data-period>
			{#each choices as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
		</select>
	</label>

	{#if error}
		<p class="error">{error}</p>
		<button class="btn" onclick={load}>Try again</button>
	{:else if !stats}
		<p class="muted" role="status">Loading…</p>
	{:else if !stats.enabled}
		<div class="card empty" data-stats-off>
			<p>Listening history is switched off, so there’s nothing to show. Switch it on and what you play from then on is counted here, only for you.</p>
			<a class="btn" href="#/settings">Listening history settings</a>
		</div>
	{:else if stats.totals.plays === 0}
		<div class="card empty" data-stats-empty>
			<p>Nothing played {words} yet.</p>
			<p class="muted">Play some music and it shows here within five minutes. Charts and streaks fill in over a few days of listening.</p>
		</div>
	{:else}
		{#if loading}<p class="muted sr-only" role="status">Updating…</p>{/if}
		<dl class="totals" aria-label={`Totals ${words}`}>
			<div><dt>Listened</dt><dd>{P.duration(stats.totals.seconds)}</dd></div>
			<div><dt>Plays</dt><dd>{P.number(stats.totals.plays)}</dd></div>
			<div><dt>Tracks</dt><dd>{P.number(stats.totals.tracks)}</dd></div>
			<div><dt>Albums</dt><dd>{P.number(stats.totals.albums)}</dd></div>
			<div><dt>Artists</dt><dd>{P.number(stats.totals.artists)}</dd></div>
		</dl>

		{#if year !== null}
			<button class="btn primary story-btn" onclick={() => (story = year)} data-open-story>Play Your year in sound {year}</button>
		{/if}

		<div class="lists">
			{#if stats.top_artists.length}
				<section aria-labelledby="top-artists">
					<h2 id="top-artists">Most played artists</h2>
					<ol class="top">
						{#each stats.top_artists.slice(0, more.artists ? 20 : 10) as a (a.id)}
							<li>
								<a href={link('artist', a.id) ?? '#/artists'}><span class="name ellipsis">{a.name || 'An unnamed artist'}</span><span class="muted small">{amount(a.plays, a.seconds)}</span></a>
							</li>
						{/each}
					</ol>
					{#if stats.top_artists.length > 10 && !more.artists}<button class="btn" onclick={() => (more.artists = true)}>Show 20</button>{/if}
				</section>
			{/if}
			{#if stats.top_albums.length}
				<section aria-labelledby="top-albums">
					<h2 id="top-albums">Most played albums</h2>
					<ol class="top">
						{#each stats.top_albums.slice(0, more.albums ? 20 : 10) as a (a.id)}
							{@const by = textOf(names.albums.get(a.id)?.artist)}
							<li>
								<a href={link('album', a.id) ?? '#/albums'}
									><span class="name ellipsis">{albumName(a.id)}</span><span class="muted small">{by ? `${by} · ` : ''}{amount(a.plays, a.seconds)}</span></a
								>
							</li>
						{/each}
					</ol>
					{#if stats.top_albums.length > 10 && !more.albums}<button class="btn" onclick={() => (more.albums = true)}>Show 20</button>{/if}
				</section>
			{/if}
			{#if stats.top_tracks.length}
				<section aria-labelledby="top-tracks">
					<h2 id="top-tracks">Most played tracks</h2>
					<ol class="top">
						{#each stats.top_tracks.slice(0, more.tracks ? 20 : 10) as t (t.id)}
							{@const tr = trackOf(t.id)}
							{@const to = trackLink(t.id)}
							{@const label = tr ? textOf(tr.title) || 'An untitled track' : 'A track no longer in the library'}
							{@const sub = `${tr && textOf(tr.artist) ? `${textOf(tr.artist)} · ` : ''}${amount(t.plays, t.seconds)}`}
							<li>
								{#if to}
									<a href={to}><span class="name ellipsis">{label}</span><span class="muted small">{sub}</span></a>
								{:else}
									<span class="row"><span class="name ellipsis">{label}</span><span class="muted small">{sub}</span></span>
								{/if}
							</li>
						{/each}
					</ol>
					{#if stats.top_tracks.length > 10 && !more.tracks}<button class="btn" onclick={() => (more.tracks = true)}>Show 20</button>{/if}
				</section>
			{/if}
		</div>

		<section aria-labelledby="new-artists">
			<h2 id="new-artists">New artists {words}</h2>
			{#if stats.new_artists.length}
				{#if since}<p class="muted small">Artists first played since your history began{oldest ? ` (it reaches back to ${oldest})` : ''}.</p>{/if}
				<ul class="chips">
					{#each stats.new_artists as a (a.id)}
						<li><a class="chip" href={link('artist', a.id) ?? '#/artists'}>{a.name || 'An unnamed artist'}</a></li>
					{/each}
				</ul>
			{:else}
				<p class="muted">No artists played for the first time {words}.</p>
			{/if}
		</section>

		<section aria-labelledby="when">
			<h2 id="when">When you listen</h2>
			<div class="charts">
				<StatsChart title="By hour" summary={P.hourSummary(stats.by_hour)} labels={HOUR_LABELS} names={HOUR_NAMES} values={stats.by_hour} unit={P.plays} every={6} />
				<StatsChart title="By day of the week" summary={P.weekdaySummary(stats.by_weekday)} labels={P.WEEKDAYS_SHORT} names={P.WEEKDAYS} values={stats.by_weekday} unit={P.plays} />
				{#if stats.by_month.length}
					<StatsChart
						title="By month"
						summary={P.monthSummary(stats.by_month)}
						labels={stats.by_month.map((m) => P.monthName(m.month).slice(0, 1))}
						names={stats.by_month.map((m) => P.monthName(m.month))}
						values={stats.by_month.map((m) => m.seconds)}
						unit={P.durationWords}
					/>
				{/if}
			</div>
		</section>

		<section aria-labelledby="streaks">
			<h2 id="streaks">Streaks</h2>
			<dl class="streaks">
				{#if stats.longest_streak}
					{@const k = stats.longest_streak}
					<div>
						<dt>Longest run of days with music</dt>
						<dd>{P.number(k.days)} {k.days === 1 ? 'day' : 'days'}<span class="muted small">{k.days === 1 ? P.dayName(k.from) : `${P.dayName(k.from)} to ${P.dayName(k.to)}`}</span></dd>
					</div>
				{/if}
				{#if stats.busiest_day}
					<div>
						<dt>Busiest day</dt>
						<dd>{P.dayName(stats.busiest_day.date)}<span class="muted small">{P.durationWords(stats.busiest_day.seconds)}</span></dd>
					</div>
				{/if}
			</dl>
		</section>
	{/if}
</section>

{#if story !== null}
	<YearStory year={story} onclose={() => (story = null)} />
{/if}

<style>
	h1 {
		margin: 0 0 0.75rem;
	}
	h2 {
		margin: 1.5rem 0 0.6rem;
	}
	.period select {
		min-height: 44px;
	}
	.empty {
		max-width: 560px;
		margin-top: 1rem;
	}
	.totals {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
		gap: 0.6rem;
		margin: 1rem 0;
	}
	.totals div {
		padding: 0.75rem 0.9rem;
		border-radius: var(--radius);
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	.totals dt {
		font-size: 0.85rem;
		color: var(--fg-muted);
	}
	.totals dd {
		margin: 0.2rem 0 0;
		font-size: 1.6rem;
		font-weight: 700;
	}
	.story-btn {
		min-height: 44px;
	}
	.lists {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
		gap: 0 1.5rem;
	}
	.top {
		margin: 0 0 0.5rem;
		padding-left: 1.6rem;
	}
	.top li {
		min-width: 0;
	}
	.top a,
	.top .row {
		display: grid;
		min-height: 44px;
		align-content: center;
		min-width: 0;
	}
	.name {
		font-weight: 600;
	}
	.small {
		font-size: 0.82rem;
	}
	.chips {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}
	.chip {
		display: inline-flex;
		align-items: center;
		min-height: 44px;
		padding: 0 0.9rem;
		border-radius: 999px;
		background: var(--bg-raised);
		border: 1px solid var(--line);
	}
	.charts {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
		gap: 1.5rem;
	}
	.streaks {
		display: grid;
		gap: 0.75rem;
		margin: 0;
	}
	.streaks dt {
		color: var(--fg-muted);
		font-size: 0.9rem;
	}
	.streaks dd {
		margin: 0.15rem 0 0;
		font-size: 1.25rem;
		font-weight: 700;
		display: grid;
	}
</style>
