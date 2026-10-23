<script lang="ts">
	import BackButton from '#lib/components/BackButton.svelte';
	import { page } from '$app/state';
	import * as cmd from '#lib/lms/commands';
	import MenuView from '#lib/components/MenuView.svelte';

	const appName = $derived.by(() => {
		try {
			return cmd.appCommand(page.params.app);
		} catch {
			return null;
		}
	});
</script>

<section class="page">
	<BackButton parent="/apps" />
	{#if appName}
		{#key appName}
			<MenuView start={cmd.appMenu(appName)} title="" />
		{/key}
	{:else}
		<p class="error">Not an app.</p>
	{/if}
</section>

<style>
</style>
