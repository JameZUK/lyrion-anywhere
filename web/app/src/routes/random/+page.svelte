<script lang="ts">
	// Random albums is Library’s Albums in the order "Random" (Wave 3.5: one Library model). This address opens Library there instead, without
	// adding a history entry, so Back skips it.
	import { go } from '$lib/nav';
	import { openSection } from '$lib/library/prefs';

	$effect(() => {
		openSection('albums', 'random');
		void go('/library', { replaceState: true });
	});
</script>
