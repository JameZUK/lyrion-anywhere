<script lang="ts">
	// The one Visualiser button on phones (Now playing's bottom row). A tap switches the
	// visualiser on or off; a long press, a right-click, Shift+F10 or the context-menu key asks
	// for the style chooser. Its label says the style while it is on. Under reduced motion it
	// is disabled, and says why.
	import Icon from './Icon.svelte';

	let {
		on,
		style,
		reduce = false,
		ontoggle,
		onchoose,
	}: {
		on: boolean;
		/** The style's name, shown while it is on. */
		style: string;
		reduce?: boolean;
		ontoggle: () => void;
		/** Open the style chooser. */
		onchoose: () => void;
	} = $props();

	/** How long a press must last to open the chooser (ms). */
	const LONG_MS = 500;
	const uid = $props.id();
	const active = $derived(on && !reduce);
	let timer: ReturnType<typeof setTimeout> | undefined;
	let longPressed = false;
	let longAt = 0;
	/** A long press happened: the click its lift may bring is not a tap. */
	function markLong() {
		longPressed = true;
		longAt = performance.now();
	}
	/** A pointer is down on the button. */
	let pressing = false;
	const justLong = () => longPressed && performance.now() - longAt < 2000;

	function down(e: PointerEvent) {
		if (reduce || (e.pointerType === 'mouse' && e.button !== 0)) return;
		longPressed = false;
		pressing = true;
		clearTimeout(timer);
		timer = setTimeout(() => {
			markLong();
			onchoose();
		}, LONG_MS);
	}
	function cancel() {
		pressing = false;
		clearTimeout(timer);
	}
	function onclick(e: MouseEvent) {
		cancel();
		const wasLong = justLong();
		longPressed = false;
		if (wasLong) {
			// The lift after a long press: the chooser is open, nothing else to do.
			e.preventDefault();
			return;
		}
		ontoggle();
	}
	function onkeydown(e: KeyboardEvent) {
		if (!reduce && ((e.key === 'F10' && e.shiftKey) || e.key === 'ContextMenu')) {
			e.preventDefault();
			onchoose();
		}
	}
	function oncontextmenu(e: MouseEvent) {
		e.preventDefault();
		const wasPressing = pressing;
		cancel();
		// A touch long press may bring this too: the chooser opens once, and the lift that
		// follows doesn't also switch the visualiser.
		if (reduce || justLong()) return;
		if (wasPressing) markLong();
		onchoose();
	}
	$effect(() => () => clearTimeout(timer));
</script>

<button
	class="np-act"
	aria-pressed={active}
	aria-label={reduce ? 'Visualiser (off while your device asks for reduced motion)' : active ? `Visualiser: ${style}` : 'Visualiser'}
	aria-describedby={`${uid}-how`}
	title={reduce ? 'Off while your device asks for reduced motion' : undefined}
	disabled={reduce}
	{onclick}
	{onkeydown}
	{oncontextmenu}
	onpointerdown={down}
	onpointerup={cancel}
	onpointerleave={cancel}
	onpointercancel={cancel}
	data-visualiser-button
>
	<Icon name="visualiser" size={24} /><span class="np-act-label">{active ? style : 'Visualiser'}</span>
</button>
<span class="sr-only" id={`${uid}-how`}>{reduce ? 'Off while your device asks for reduced motion.' : 'Tap to switch on or off. Long-press, or press Shift+F10, to choose a style.'}</span>
