<script lang="ts">
	// Settings › Ratings: stars or a heart, and clearing every rating (after asking).
	import type { RatingStyle } from '$lib/api/routine';
	import { ratings } from '$lib/state/ratings';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import PickerRow from './PickerRow.svelte';

	let note = $state('');
	let confirmClear = $state(false);

	$effect(() => {
		void ratings.load([]);
	});

	async function setStyle(style: RatingStyle) {
		note = '';
		try {
			await ratings.setStyle(style);
		} catch (e) {
			note = `Couldn’t change how you rate: ${(e as Error).message}`;
		}
	}

	async function clear() {
		confirmClear = false;
		note = '';
		try {
			const n = await ratings.clearAll();
			note = n === 1 ? '1 rating cleared.' : `${n} ratings cleared.`;
		} catch (e) {
			note = `Couldn’t clear your ratings: ${(e as Error).message}`;
		}
	}
</script>

<div data-rating-style={ratings.style}>
	<PickerRow
		label="Rate with"
		value={ratings.style}
		options={[
			{ value: 'stars', label: 'Stars' },
			{ value: 'heart', label: 'A heart' },
		]}
		onchange={(v) => void setStyle(v === 'heart' ? 'heart' : 'stars')}
		note="Kept for your account, and shared with Ratings Light where your music server has it."
		id="rating-style"
	/>
</div>
<button class="btn" onclick={() => (confirmClear = true)} data-clear-ratings>Clear my ratings</button>
<p class="muted small" role="status" data-ratings-note>{note}</p>
{#if confirmClear}
	<ConfirmDialog
		title="Clear your ratings?"
		message="Every rating you have given is removed from your account. This can’t be undone."
		confirmLabel="Clear ratings"
		danger
		onconfirm={clear}
		oncancel={() => (confirmClear = false)}
	/>
{/if}

<style>
	.small {
		font-size: 0.85rem;
		margin: 0.5rem 0;
	}
</style>
