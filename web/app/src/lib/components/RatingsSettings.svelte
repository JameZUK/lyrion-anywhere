<script lang="ts">
	// Settings › Ratings: stars or a heart, and clearing every rating (after asking).
	import type { RatingStyle } from '$lib/api/routine';
	import { ratings } from '$lib/state/ratings';
	import ConfirmDialog from './ConfirmDialog.svelte';

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

<label class="field">
	<span>Rate with</span>
	<select value={ratings.style} onchange={(e) => void setStyle(e.currentTarget.value === 'heart' ? 'heart' : 'stars')} data-rating-style>
		<option value="stars">Stars</option>
		<option value="heart">A heart</option>
	</select>
</label>
<p class="muted small">
	Your ratings are yours: kept on your server for your account and used for your shelves and stations. Where the server has the Ratings Light plugin, your rating also becomes its shared rating for the
	track.
</p>
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
