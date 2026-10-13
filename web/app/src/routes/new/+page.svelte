<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import AlbumBrowser from '$lib/components/AlbumBrowser.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';
</script>

<section class="page">
	<BackButton parent="/library" />
	<h1>New Music</h1>
	<p class="muted lead">Albums most recently added to the library, newest first.</p>
	<LibraryNote />
	<AlbumBrowser sort="new" sorts={null} empty="Nothing has been added to the library yet." />
</section>

<style>
	h1 {
		margin-bottom: 0.2rem;
	}
	.lead {
		margin: 0 0 0.75rem;
	}
</style>
