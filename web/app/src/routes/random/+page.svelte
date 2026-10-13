<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import AlbumBrowser from '$lib/components/AlbumBrowser.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let version = $state(0);
</script>

<section class="page">
	<BackButton parent="/library" />
	<header>
		<h1>Random albums</h1>
		<button class="btn" onclick={() => version++}><Icon name="shuffle" size={18} />Shuffle again</button>
	</header>
	<LibraryNote />
	<!-- One batch: paging a random order would only repeat albums. -->
	<AlbumBrowser sort="random" sorts={null} paged={false} pageSize={60} {version} empty="No albums in the library yet." />
</section>

<style>
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
		margin-bottom: 0.5rem;
	}
	h1 {
		margin: 0;
	}
</style>
