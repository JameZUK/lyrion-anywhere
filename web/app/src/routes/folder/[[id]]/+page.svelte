<script lang="ts">
	import { page } from '$app/state';
	import FolderView from '$lib/components/FolderView.svelte';
</script>

<section class="page">
	<FolderView id={page.params.id} />
</section>
