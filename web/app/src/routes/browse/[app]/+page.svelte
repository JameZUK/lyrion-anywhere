<script lang="ts">
	import { page } from '$app/state';
	import * as cmd from '$lib/lms/commands';
	import MenuView from '$lib/components/MenuView.svelte';

	const appName = $derived.by(() => {
		try {
			return cmd.appCommand(page.params.app);
		} catch {
			return null;
		}
	});
</script>

<section class="page">
	<a class="back muted" href="#/apps">‹ Apps &amp; Radio</a>
	{#if appName}
		{#key appName}
			<MenuView start={cmd.appMenu(appName)} title="" />
		{/key}
	{:else}
		<p class="error">Not an app.</p>
	{/if}
</section>

<style>
	.back {
		display: inline-block;
		margin-bottom: 0.5rem;
	}
</style>
