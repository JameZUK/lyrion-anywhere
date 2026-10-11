<script lang="ts">
	// A reminder, on browse pages, that only one virtual library is being shown.
	import { library } from '$lib/state/library.svelte';
</script>

{#if library.name}
	<p class="note">
		<span class="muted">In</span> <strong>{library.name}</strong>
		<button class="link" onclick={() => library.choose(null)}>Show the whole library</button>
	</p>
{/if}

<style>
	.note {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.35rem;
		margin: 0.25rem 0 0.75rem;
		font-size: 0.9rem;
	}
	.link {
		border: 0;
		background: none;
		padding: 0;
		margin-left: 0.5rem;
		color: var(--accent);
		font: inherit;
		text-decoration: underline;
		cursor: pointer;
	}
</style>
