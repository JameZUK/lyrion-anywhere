<script lang="ts">
	// A peek at the queue from Now playing (Wave 2 §3.4), in a sheet (see QueueList).
	import QueueList from './QueueList.svelte';
	import Sheet from './Sheet.svelte';

	let { onclose }: { onclose: () => void } = $props();
</script>

<Sheet title="Queue" {onclose}>
	<QueueList onplay={onclose} />
</Sheet>
