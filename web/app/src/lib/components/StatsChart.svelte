<script lang="ts">
	// A small bar chart for the stats page. The drawing is decoration (hidden from screen
	// readers); its text alternative is the summary sentence under it and a table of every
	// value, visually hidden.
	let {
		title,
		summary,
		labels,
		names,
		values,
		unit,
		every = 1,
	}: {
		title: string;
		/** One sentence saying what the chart shows. */
		summary: string;
		/** Short labels under the bars. */
		labels: string[];
		/** Full names for the table ("Monday", "21:00–22:00"). */
		names: string[];
		values: number[];
		/** How a value reads in the table ("12 plays"). */
		unit: (v: number) => string;
		/** Label every n-th bar (busy charts). */
		every?: number;
	} = $props();

	const id = `chart-${Math.random().toString(36).slice(2, 9)}`;
	const max = $derived(Math.max(1, ...values));
	const W = 10;
	const H = 60;
</script>

<figure class="chart" aria-labelledby={`${id}-t`} data-chart>
	<h3 id={`${id}-t`}>{title}</h3>
	<svg viewBox={`0 0 ${values.length * W} ${H}`} preserveAspectRatio="none" aria-hidden="true" focusable="false">
		{#each values as v, i (i)}
			{@const bh = v > 0 ? Math.max(1.5, (v / max) * H) : 0}
			<rect x={i * W + 1.5} y={H - bh} width={W - 3} height={bh} rx="1.5" class:top={v === max && v > 0} />
		{/each}
	</svg>
	<div class="labels" aria-hidden="true" style={`grid-template-columns: repeat(${values.length}, 1fr)`}>
		{#each labels as l, i (i)}<span>{i % every === 0 ? l : ''}</span>{/each}
	</div>
	<table class="sr-only">
		<caption>{title}</caption>
		<thead><tr><th scope="col">When</th><th scope="col">Listening</th></tr></thead>
		<tbody>
			{#each values as v, i (i)}
				<tr><th scope="row">{names[i] ?? labels[i]}</th><td>{unit(v)}</td></tr>
			{/each}
		</tbody>
	</table>
	<figcaption class="muted">{summary}</figcaption>
</figure>

<style>
	.chart {
		margin: 0;
		min-width: 0;
	}
	h3 {
		font-size: 1rem;
		margin: 0 0 0.5rem;
	}
	svg {
		display: block;
		width: 100%;
		height: 120px;
	}
	rect {
		fill: color-mix(in srgb, var(--accent) 45%, transparent);
	}
	rect.top {
		fill: var(--accent);
	}
	.labels {
		display: grid;
		font-size: 0.72rem;
		color: var(--fg-muted);
		text-align: center;
		margin-top: 0.25rem;
	}
	.labels span {
		overflow: visible;
		white-space: nowrap;
	}
	figcaption {
		margin-top: 0.4rem;
		font-size: 0.9rem;
	}
</style>
