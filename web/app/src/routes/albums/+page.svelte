<script lang="ts">
	import type { AlbumSort } from '$lib/lms/commands';
	import AlbumBrowser, { ALBUM_SORTS } from '$lib/components/AlbumBrowser.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';

	let sort = $state<AlbumSort>('album');
</script>

<section class="page">
	<h1>Albums</h1>
	<LibraryNote />
	<AlbumBrowser bind:sort sorts={[...ALBUM_SORTS, 'random']} empty="No albums in the library yet." />
</section>
