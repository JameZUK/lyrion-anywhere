<script lang="ts">
	// The signal path, opened from the quality badge: source, decoding, volume and output.
	import Sheet from './Sheet.svelte';
	import SignalPathView from './SignalPathView.svelte';

	let { onclose }: { onclose: () => void } = $props();
</script>

<Sheet title="Signal path" {onclose}>
	<SignalPathView />
</Sheet>
