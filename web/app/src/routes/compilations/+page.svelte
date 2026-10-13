<script lang="ts">
	import BackButton from '$lib/components/BackButton.svelte';
	import type { AlbumSort } from '$lib/lms/commands';
	import AlbumBrowser from '$lib/components/AlbumBrowser.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';

	let sort = $state<AlbumSort>('album');
</script>

<section class="page">
	<BackButton parent="/library" />
	<h1>Compilations</h1>
	<LibraryNote />
	<AlbumBrowser bind:sort filter={{ compilation: true }} sorts={['album', 'yearalbum', 'new']} showArtist={false} empty="No compilations in the library." />
</section>
