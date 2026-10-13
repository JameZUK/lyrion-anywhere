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
		// An evenly spaced 8-tooth gear centred on the grid (the hand-drawn one was lopsided).
		settings: 'M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6zM10.26 5.01L10.42 2.53L13.58 2.53L13.74 5.01L15.71 5.83L17.57 4.18L19.82 6.43L18.17 8.29L18.99 10.26L21.47 10.42L21.47 13.58L18.99 13.74L18.17 15.71L19.82 17.57L17.57 19.82L15.71 18.17L13.74 18.99L13.58 21.47L10.42 21.47L10.26 18.99L8.29 18.17L6.43 19.82L4.18 17.57L5.83 15.71L5.01 13.74L2.53 13.58L2.53 10.42L5.01 10.26L5.83 8.29L4.18 6.43L6.43 4.18L8.29 5.83Z',
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
		heart: 'M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z',
		radio: 'M4 9h16v11H4zM8 5l9 4M8.5 14.5a2 2 0 1 0 0 .01M14 13h3M14 16h3',
		library: 'M5 4v16M10 4v16M14.5 4.5l4.5 15',
		download: 'M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19.5h14',
		check: 'M5 12.5l4.5 4.5L19 7.5',
		new: 'M12 3.5l1.9 5.1 5.1 1.9-5.1 1.9L12 17.5l-1.9-5.1L5 10.5l5.1-1.9zM18.5 15.5l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8z',
		genre: 'M4 4h7.5L20 12.5 12.5 20 4 11.5zM8 8h.01',
		calendar: 'M5 6h14v14H5zM5 10h14M9 3.5v4M15 3.5v4',
		folder: 'M3.5 6.5a1 1 0 0 1 1-1H9l2 2.5h8.5a1 1 0 0 1 1 1V18a1 1 0 0 1-1 1h-15a1 1 0 0 1-1-1z',
		stack: 'M4 8l8-4 8 4-8 4zM4 12l8 4 8-4M4 16l8 4 8-4',
		note: 'M9 18V6l10-2v12M9 18a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0zM19 16a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0z',
		// Cast-style: a screen with waves from its corner (where the sound plays).
		output: 'M3.5 8V6a1 1 0 0 1 1-1h15a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1h-6M3.5 11.5a7.5 7.5 0 0 1 7.5 7.5M3.5 15a4 4 0 0 1 4 4M3.5 19h.01',
		transfer: 'M4 12h14M13 7l5 5-5 5',
		// Home: shelves of albums.
		shelves: 'M4 4.5h7v7H4zM13 4.5h7v7h-7zM4 15.5h16M4 19.5h16',
		grip: 'M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01',
		up: 'M12 19V5M6 11l6-6 6 6',
		down: 'M12 5v14M6 13l6 6 6-6',
		moon: 'M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z',
		lyrics: 'M4 5h16v11H9.5L5 20v-4H4zM8 9h8M8 12.5h5',
		alarm: 'M12 8v4.5l3 2M12 21a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM5 3 2 6M19 3l3 3',
		link: 'M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1',
		offline:'M3 8.5a13 13 0 0 1 18 0M6.5 12a8 8 0 0 1 11 0M10 15.5a3 3 0 0 1 4 0M4 4l16 16',
		visualiser: 'M4 20v-5M8 20V9M12 20V4M16 20v-8M20 20v-4',
		rooms: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
		art: 'M4 4h16v16H4zM4 16l5-5 4 4 2-2 5 5M15.5 8.5h.01',
		info: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 11v6M12 7.5h.01',
		car: 'M5 16.5V12l2-5h10l2 5v4.5zM3.5 12h17M7.5 14.5h.01M16.5 14.5h.01M6 16.5V19M18 16.5V19',
		filter: 'M4 5h16l-6 7.5V19l-4-2v-4.5z',
		keyboard: 'M3 6.5h18v11H3zM6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7.5 14h9',
		stop: 'M6.5 6.5h11v11h-11z',
	};
	const SOLID = new Set(['play', 'pause', 'next', 'prev', 'stop']);

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
