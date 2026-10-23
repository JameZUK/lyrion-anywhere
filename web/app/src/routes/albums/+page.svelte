<script lang="ts">
	// Albums is a section of Library (Wave 3.5: one Library model). This address opens Library there instead, without
	// adding a history entry, so Back skips it.
	import { go } from '#lib/nav';
	import { openSection } from '#lib/library/prefs';

	$effect(() => {
		openSection('albums');
		void go('/library', { replaceState: true });
	});
</script>
