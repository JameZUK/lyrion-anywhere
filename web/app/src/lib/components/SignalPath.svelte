<script lang="ts">
	// The signal path, opened from the quality badge: source, decoding, volume and output. On a
	// wide screen the sheet is a wider dialog (760 px) and the blocks sit in a row, with the
	// quality switch on its own line beneath; on a phone it is a bottom sheet and they stack.
	import Sheet from './Sheet.svelte';
	import SignalPathView from './SignalPathView.svelte';

	let { onclose }: { onclose: () => void } = $props();

	/** Wider than Sheet's phone breakpoint (760 px): a centred dialog with room for a row. */
	const ROW_QUERY = '(min-width: 761px)';
	let row = $state(false);
	$effect(() => {
		const mq = typeof matchMedia === 'function' ? matchMedia(ROW_QUERY) : null;
		const update = () => (row = !!mq?.matches);
		update();
		mq?.addEventListener?.('change', update);
		return () => mq?.removeEventListener?.('change', update);
	});
</script>

<Sheet title="Signal path" wide {onclose}>
	<SignalPathView {row} />
</Sheet>
