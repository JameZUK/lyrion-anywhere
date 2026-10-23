<script lang="ts">
	// Back, at the top of detail pages: an installed iPhone web app has no browser Back. It
	// returns to wherever the user came from, or (opened directly) up to `parent`. Without
	// script it is a plain link to the parent.
	import { back, href } from '#lib/nav';
	import Icon from './Icon.svelte';

	let { parent }: { parent: string } = $props();

	function onclick(e: MouseEvent) {
		if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
		e.preventDefault();
		void back(parent);
	}
</script>

<a class="back-btn" href={href(parent)} {onclick} data-back><Icon name="back" size={20} /><span>Back</span></a>
