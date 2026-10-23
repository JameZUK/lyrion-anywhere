<script lang="ts">
	import { page } from '$app/state';
	import FavouritesView from '#lib/components/FavouritesView.svelte';
</script>

<section class="page">
	<FavouritesView item={page.params.item} />
</section>
