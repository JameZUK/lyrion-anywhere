<script lang="ts">
	import type { AlbumSort } from '$lib/lms/commands';
	import AlbumBrowser, { ALBUM_SORTS } from '$lib/components/AlbumBrowser.svelte';
	import LibraryNote from '$lib/components/LibraryNote.svelte';
	import { loadLibraryPrefs, saveLibraryPrefs, type AlbumView } from '$lib/library/prefs';

	// The grid or list choice is Library's, remembered on this device.
	const saved = loadLibraryPrefs();
	let sort = $state<AlbumSort>('album');
	let view = $state<AlbumView>(saved.view);
	$effect(() => {
		const v = view;
		const now = loadLibraryPrefs();
		if (now.view !== v) saveLibraryPrefs({ ...now, view: v });
	});
</script>

<section class="page">
	<h1>Albums</h1>
	<LibraryNote />
	<AlbumBrowser bind:sort bind:view views az sorts={[...ALBUM_SORTS, 'random']} empty="No albums in the library yet." />
</section>
