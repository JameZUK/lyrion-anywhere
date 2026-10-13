<script lang="ts">
	// A biography or review: a heading, then the text as plain text (never markup). Long text
	// shows its first lines with a "Read more" disclosure button.
	import { isLong, paragraphs } from '$lib/info/info';

	let { heading, text }: { heading: string; text: string } = $props();

	const uid = $props.id();
	let open = $state(false);
	const parts = $derived(paragraphs(text));
	const long = $derived(isLong(text));
	const shown = $derived(open || !long ? parts : parts.slice(0, 1));
</script>

<section class="info" aria-labelledby={`${uid}-h`} data-info={heading}>
	<h2 id={`${uid}-h`}>{heading}</h2>
	<div id={`${uid}-text`} class="text" class:clamped={long && !open}>
		{#each shown as p, i (i)}<p>{p}</p>{/each}
	</div>
	{#if long}
		<button
			class="more"
			type="button"
			aria-expanded={open}
			aria-controls={`${uid}-text`}
			aria-label={open ? 'Show less' : `Read more of the ${heading.toLowerCase()}`}
			onclick={() => (open = !open)}
		>
			{open ? 'Show less' : 'Read more'}
		</button>
	{/if}
</section>

<style>
	.info {
		max-width: 70ch;
		margin: 0 0 1.75rem;
	}
	h2 {
		font-size: 1rem;
		margin: 0 0 0.4rem;
	}
	.text p {
		margin: 0 0 0.7rem;
		white-space: pre-line;
		line-height: 1.55;
	}
	.text.clamped p {
		display: -webkit-box;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		overflow: hidden;
		margin: 0;
	}
	.more {
		min-height: 44px;
		padding: 0 0.2rem;
		border: none;
		background: none;
		color: var(--accent);
		font: inherit;
		font-weight: 600;
		cursor: pointer;
	}
	.more:hover {
		text-decoration: underline;
	}
</style>
