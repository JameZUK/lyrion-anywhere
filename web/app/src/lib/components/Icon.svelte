<script lang="ts">
	// Simple line/solid icons on a 24-unit grid. Decorative unless given a label.
	const PATHS: Record<string, string> = {
		play: 'M8 5.5v13a1 1 0 0 0 1.5.86l10.4-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z',
		pause: 'M7 5h3.5v14H7zM13.5 5H17v14h-3.5z',
		next: 'M6 6.5v11a.8.8 0 0 0 1.25.66l7.5-5.5a.8.8 0 0 0 0-1.32l-7.5-5.5A.8.8 0 0 0 6 6.5zM16 6h2v12h-2z',
		prev: 'M18 6.5v11a.8.8 0 0 1-1.25.66l-7.5-5.5a.8.8 0 0 1 0-1.32l7.5-5.5A.8.8 0 0 1 18 6.5zM6 6h2v12H6z',
		volume: 'M4 9.5h3.5L12 5.5v13l-4.5-4H4zM15.5 8.5a5 5 0 0 1 0 7M18 6a8.5 8.5 0 0 1 0 12',
		mute: 'M4 9.5h3.5L12 5.5v13l-4.5-4H4zM16 9.5l5 5M21 9.5l-5 5',
		search: 'M10.5 4a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM15.5 15.5 20 20',
		album: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 9.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5z',
		artist: 'M12 4a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM4.5 20a7.5 7.5 0 0 1 15 0',
		queue: 'M4 6h12M4 11h12M4 16h7M15 14.5v6l5-3z',
		settings: 'M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6zM19.4 13.5l1.6 1.2-2 3.5-1.9-.7a7 7 0 0 1-1.7 1l-.3 2h-4l-.3-2a7 7 0 0 1-1.7-1l-1.9.7-2-3.5 1.6-1.2a7 7 0 0 1 0-3L3.2 9.3l2-3.5 1.9.7a7 7 0 0 1 1.7-1l.3-2h4l.3 2a7 7 0 0 1 1.7 1l1.9-.7 2 3.5-1.6 1.2a7 7 0 0 1 0 3z',
		speaker: 'M7 3h10a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM12 11a3.5 3.5 0 1 1 0 7 3.5 3.5 0 0 1 0-7zM12 6.5h.01',
		device: 'M8 2.5h8a1.5 1.5 0 0 1 1.5 1.5v16a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 20V4A1.5 1.5 0 0 1 8 2.5zM11 18.5h2',
		more: 'M12 5.5h.01M12 12h.01M12 18.5h.01',
		close: 'M6 6l12 12M18 6 6 18',
		shuffle: 'M4 7h3.5c4 0 5 10 9 10H20M4 17h3.5c1.5 0 2.5-1.4 3.3-3M14 7h6M17.5 4.5 20 7l-2.5 2.5M17.5 14.5 20 17l-2.5 2.5',
		repeat: 'M5 11V9a3 3 0 0 1 3-3h11M16 3l3 3-3 3M19 13v2a3 3 0 0 1-3 3H5M8 21l-3-3 3-3',
		plus: 'M12 5v14M5 12h14',
		playnext: 'M4 6h10M4 11h10M4 16h6M15 13v7l5.5-3.5z',
		home: 'M4 11 12 4l8 7v8.5a.5.5 0 0 1-.5.5H15v-6H9v6H4.5a.5.5 0 0 1-.5-.5z',
		back: 'M15 5l-7 7 7 7',
		chevron: 'M6 9l6 6 6-6',
		trash: 'M5 7h14M10 7V4.5h4V7M7 7l1 13h8l1-13',
		power: 'M12 3v8M7 6.5a7 7 0 1 0 10 0',
	};
	const SOLID = new Set(['play', 'pause', 'next', 'prev']);

	let { name, size = 22, label = '' }: { name: keyof typeof PATHS | string; size?: number; label?: string } = $props();
	const d = $derived(PATHS[name] ?? '');
	const solid = $derived(SOLID.has(name));
</script>

<svg
	width={size}
	height={size}
	viewBox="0 0 24 24"
	aria-hidden={label ? undefined : 'true'}
	role={label ? 'img' : undefined}
	aria-label={label || undefined}
	fill={solid ? 'currentColor' : 'none'}
	stroke={solid ? 'none' : 'currentColor'}
	stroke-width="1.8"
	stroke-linecap="round"
	stroke-linejoin="round"><path {d} /></svg
>
