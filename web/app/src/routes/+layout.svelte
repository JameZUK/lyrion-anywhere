<script lang="ts">
	import '../app.css';
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { afterNavigate } from '$app/navigation';
	import { noteNavigation, routeOutsideHashChanges } from '$lib/nav';
	import { MAIN_TABS, tabFor } from '$lib/nav-tabs';
	import { documentTitle, pageTitle } from '$lib/title.svelte';
	import { api, app, session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import { appearance } from '$lib/theme/appearance.svelte';
	import { install } from '$lib/pwa/install.svelte';
	import { clearResume, loadResume, planResume, saveResume, trackId } from '$lib/player/resume';
	import type { Command } from '$lib/api/rpc';
	import Login from '$lib/components/Login.svelte';
	import PlayerBar from '$lib/components/PlayerBar.svelte';
	import MediaSession from '$lib/components/MediaSession.svelte';
	import KeyboardShortcuts from '$lib/components/KeyboardShortcuts.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import OfflineBar from '$lib/components/OfflineBar.svelte';
	import OfflineMode from '$lib/components/OfflineMode.svelte';
	import { downloads, handoff, offlinePlayer, plays } from '$lib/offline/index.svelte';
	import { snapshotOf } from '$lib/offline/handoff.svelte';
	import ContinueOn from '$lib/components/ContinueOn.svelte';
	import { forgetOwner, rememberOwner } from '$lib/offline/settings';
	import { library } from '$lib/state/library.svelte';
	import { controls, local } from '$lib/player/controls.svelte';
	import { CONNECTION_ID } from '$lib/state/reachability';
	// Keeps a playing station topped up wherever the user goes.
	import '$lib/stations/active.svelte';

	let { children } = $props();

	/** How long the app waits for this device's player to appear in LMS before using another. */
	const DEVICE_WAIT_MS = 20_000;

	// The chosen theme, before anything is drawn (also on the sign-in page).
	appearance.start();
	install.start();

	// A returning browser signs in silently through its refresh cookie.
	void session.restore();

	// Diagnostics for troubleshooting: report once signed in (capabilities, playback choice).
	$effect(() => {
		// Not before the server has confirmed the sign-in: the player needs a real token.
		if (app.status === 'signed-in' && !app.provisional) {
			// Test the media-element path quietly, so this device can use it at once. Then start
			// this device's player by itself if it is the one to use (SPEC D20: player-first), or
			// if it was playing before a reload: it connects now, and the first tap anywhere
			// turns the sound on.
			void device.prepare().then(async () => {
				device.report();
				if (!untrack(() => app.wantDevice) && !device.wasActive) return;
				const r = await device.autoStart();
				// Another tab of this browser runs it: control that (once LMS lists it).
				if (r === 'busy') void app.waitForPlayer(app.deviceMac);
				else if (r !== 'started') app.deviceFailed();
			});
		}
	});

	// This device's player failed while the app was waiting for it, or LMS still hasn't listed
	// it after a while (e.g. the server can't reach back): use another player, and say why.
	const waitingForDevice = $derived(!!app.playerId && app.playerId === app.deviceMac && !app.current);
	$effect(() => {
		// Waiting for the server to confirm the sign-in first: the player hasn't started yet.
		if (!waitingForDevice || app.provisional) return;
		if (device.state === 'error') {
			untrack(() => app.deviceFailed());
			return;
		}
		const t = setTimeout(() => app.deviceFailed(), DEVICE_WAIT_MS);
		return () => clearTimeout(t);
	});

	// The sound is held back until a tap, but only worth saying while this device plays.
	const heldBack = $derived(device.needsTap && (device.detail.output === 'Playing' || (app.playerId === device.mac && app.playing)));

	// Downloads belong to the signed-in user. Remember who that is, so offline mode can offer
	// their downloads when the server can't be reached; forget it once signed out.
	let lastStatus = '';
	$effect(() => {
		const user = app.session?.user.username ?? null;
		const was = lastStatus;
		lastStatus = app.status;
		if (app.status === 'offline') handoff.wentOffline();
		if (app.status === 'signed-in' && user) {
			rememberOwner(user);
			// Downloads carry on once the server has confirmed the sign-in.
			void downloads.setUser(user, !app.provisional);
			void plays.setUser(user, true);
			void library.start(user, (c) => api.rpc('', c));
			// Back from offline with downloads queued: offer "Continue on <player>" (SPEC §8.4).
			if (navigator.onLine !== false) untrack(() => handoff.backOnline(user, snapshotOf(offlinePlayer)));
		} else if (app.status === 'signed-out') {
			// The sign-in ended while offline (not signed out on purpose): keep the queue to
			// offer once the same user signs in again.
			if (was === 'offline' || !session.signOutPending) untrack(() => handoff.keep(snapshotOf(offlinePlayer)));
			else handoff.reset();
			offlinePlayer.stop();
			library.stop();
			forgetOwner();
			void downloads.setUser(null);
			void plays.setUser(null);
		}
	});

	// The browser lost its connection while signed in, and has it back: the same offer.
	$effect(() => {
		const online = () => {
			const user = app.session?.user.username;
			if (app.status === 'signed-in' && user) handoff.backOnline(user, snapshotOf(offlinePlayer));
		};
		addEventListener('online', online);
		return () => removeEventListener('online', online);
	});

	// Note where this device is in the queue when the page goes away, to carry on after a reload.
	function rememberPosition() {
		// What the selected player is doing, shown after a reload until the server answers.
		app.rememberNow();
		const now = app.now;
		const index = Number(now?.playlist_cur_index);
		if (!device.mac || app.playerId !== device.mac || !now || !Number.isFinite(index)) return;
		// `status - 1` lists just the current track: its id identifies it in the queue.
		const track = trackId(now.playlist_loop?.[0]?.id);
		saveResume({ mac: device.mac, playing: now.mode === 'play', index, time: app.elapsed, at: Date.now(), ...(track !== undefined ? { track } : {}) });
	}
	$effect(() => {
		addEventListener('pagehide', rememberPosition);
		const hidden = () => document.visibilityState === 'hidden' && rememberPosition();
		document.addEventListener('visibilitychange', hidden);
		return () => {
			removeEventListener('pagehide', rememberPosition);
			document.removeEventListener('visibilitychange', hidden);
		};
	});

	// Out of reach (SPEC: controls while the server is out of reach). This device's own
	// connection counts while it is the player; then it plays on by itself, and tells the
	// server what it did once it is back.
	$effect(() => {
		app.setDeviceDown(controls.thisDevice && device.state === 'reconnecting');
	});
	$effect(() => {
		const online = app.reach === 'online';
		const on = !online && controls.thisDevice && app.status === 'signed-in';
		untrack(() => local.sync(on, online));
	});
	$effect(() => {
		void device.detail;
		untrack(() => local.onDevice());
	});
	// While in reach, keep the queue around the current track: what this device can play
	// with no server is worked out from it.
	const curIndex = $derived(Number(app.now?.playlist_cur_index ?? -1));
	$effect(() => {
		const mac = device.mac;
		const cur = curIndex;
		void app.queueVersion;
		if (app.reach !== 'online' || !controls.thisDevice || app.status !== 'signed-in' || cur < 0) return;
		const t = setTimeout(() => void local.snapshot(mac, cur).catch(() => {}), 1000);
		return () => clearTimeout(t);
	});

	// A sleep timer on this device fades its own output before the end.
	app.onDeviceSleep = (endsAt, fadeS) => device.setSleepFade(endsAt, fadeS);

	// When this browser becomes a player, control it.
	device.onConnected = async (mac) => {
		const point = loadResume();
		clearResume();
		if (!app.playerId || app.playerId === mac) {
			// LMS may not list the new player for a moment: wait for it, then select it.
			if (!(await app.waitForPlayer(mac))) return;
		} else {
			// The user controls another player (e.g. one chosen before a reload): keep it.
			await app.loadPlayers();
		}
		if (point?.mac !== mac) return;
		// After a reload: if LMS hasn't carried on playing by itself, go back to where it was.
		await new Promise((r) => setTimeout(r, 1500));
		// Ask about this device's own player: the user may have selected another meanwhile.
		let commands: Command[];
		try {
			commands = await planResume(point, (player, c) => api.rpc(player, c));
		} catch {
			return; // the user can still press play
		}
		for (const c of commands) {
			try {
				await api.rpc(mac, c);
			} catch {
				break; // the user can still press play
			}
		}
		app.soon();
	};

	// The same four destinations lead on every screen size (and in the Android app): Home,
	// Library, Search and Queue. Now playing opens from the mini player; Settings from the
	// gear on Home (and the sidebar). See lib/nav-tabs.ts for what each one covers.
	const NAV = [
		{ href: '#/home', label: 'Home', icon: 'shelves', match: (p: string) => p.startsWith('/home') || p.startsWith('/stations') },
		{ href: '#/library', label: 'Library', icon: 'library', match: (p: string) => MORE_LIBRARY.some((l) => p.startsWith(l)) },
		{ href: '#/search', label: 'Search', icon: 'search', match: (p: string) => tabFor(p) === 'search' },
		{ href: '#/queue', label: 'Queue', icon: 'playnext', match: (p: string) => tabFor(p) === 'queue' },
		{ href: '#/', label: 'Now playing', icon: 'home', match: (p: string) => p === '/' },
		{ href: '#/new', label: 'New music', icon: 'new', match: (p: string) => p.startsWith('/new') },
		{ href: '#/albums', label: 'Albums', icon: 'album', match: (p: string) => p.startsWith('/album') },
		{ href: '#/artists', label: 'Artists', icon: 'artist', match: (p: string) => p.startsWith('/artist') },
		{ href: '#/playlists', label: 'Playlists', icon: 'queue', match: (p: string) => p.startsWith('/playlist') },
		{ href: '#/favourites', label: 'Favourites', icon: 'heart', match: (p: string) => p.startsWith('/favourites') },
		{ href: '#/apps', label: 'Apps & radio', icon: 'radio', match: (p: string) => p.startsWith('/apps') || p.startsWith('/browse') || p.startsWith('/plugin') },
		{ href: '#/rooms', label: 'Rooms', icon: 'rooms', match: (p: string) => p.startsWith('/rooms') },
		{ href: '#/downloads', label: 'Downloads', icon: 'download', match: (p: string) => p.startsWith('/downloads') },
		{ href: '#/settings', label: 'Settings', icon: 'settings', match: (p: string) => p.startsWith('/settings') },
	];
	// Library sections without a sidebar entry of their own: under "Library".
	const MORE_LIBRARY = ['/library', '/genre', '/year', '/folder', '/compilations', '/random', '/filter'];
	// Phones: the four tabs.
	const TABS = MAIN_TABS.map((t) => ({ ...t, match: (p: string) => tabFor(p) === t.id }));
	const path = $derived(page.url.hash.replace(/^#/, '') || '/');

	// Back on detail pages needs to know whether there is an app page behind this one.
	afterNavigate((n) => noteNavigation({ type: n.type, delta: n.delta }));
	// A route typed into the address bar opens in the app, rather than reloading the page.
	$effect(() => routeOutsideHashChanges(() => page.url.hash));
</script>

<svelte:head>
	<!-- SvelteKit announces the new title after each navigation. -->
	<title>{app.status === 'signed-in' ? documentTitle(path, pageTitle.detail) : 'Lyrion Anywhere'}</title>
</svelte:head>

{#if app.status === 'unknown'}
	<div class="splash" aria-busy="true"><span class="dot"></span></div>
{:else if app.status === 'offline'}
	<OfflineMode />
{:else if app.status === 'signed-out'}
	<Login />
{:else}
	<MediaSession />
	<KeyboardShortcuts />
	{#if path === '/car'}
		<!-- Car mode fills the screen: no navigation, player bar or tabs around it. -->
		<main class="car-shell">{@render children()}</main>
	{:else}
		<div class="shell">
			<nav class="side" aria-label="Main">
				<div class="brand"><img class="logo" src="./icons/icon.svg" alt="" width="28" height="28" />Lyrion Anywhere</div>
				{#each NAV as n (n.href)}
					<a href={n.href} class:active={n.match(path)} aria-current={n.match(path) ? 'page' : undefined}>
						<Icon name={n.icon} size={20} />{n.label}
					</a>
				{/each}
			</nav>
			<main class="content" class:has-strip={!!app.moving && !offlinePlayer.active} class:on-np={path === '/' && !offlinePlayer.active}>
				{#if heldBack}
					<button class="banner resume tap" onclick={() => {}}>
						<Icon name="play" size={16} /><span>Playing on this device. Tap here (or anywhere) to turn the sound on.</span>
					</button>
				{:else if device.wasActive && (device.state === 'off' || device.state === 'stopped' || device.state === 'error')}
					<div class="banner resume" role="status">
						<span>This device was playing before the page reloaded.</span>
						<button class="btn primary" onclick={() => device.start()}><Icon name="play" size={16} />Resume playing here</button>
						<button class="btn" onclick={() => device.stop()}>Dismiss</button>
					</div>
				{/if}
				<!-- One connection message, at every width, taking its own room (never over the controls). -->
				{#if controls.message}
					<p class="chip" class:busy={app.reach === 'busy'} role="status" id={CONNECTION_ID} data-connection={app.reach}>
						<Icon name={app.reach === 'busy' ? 'clock' : 'offline'} size={16} /><span>{controls.message}</span>
					</p>
				{/if}
				<ContinueOn />
				{#if app.notice}
					<div class="banner resume notice" role="status">
						<span>{app.notice}</span>
						<button class="btn" onclick={() => (app.notice = '')}>OK</button>
					</div>
				{/if}
				{#if app.error}<p class="banner error" role="alert">{app.error}</p>{/if}
				{@render children()}
			</main>
		</div>
		{#if offlinePlayer.active}
			<div class="barwrap"><OfflineBar /></div>
		{:else}
			<div class="barwrap" class:on-np={path === '/'}><PlayerBar /></div>
		{/if}
		<nav class="tabs" aria-label="Main">
			{#each TABS as n (n.href)}
				<a href={n.href} class:active={n.match(path)} aria-current={n.match(path) ? 'page' : undefined}>
					<span class="ico"><Icon name={n.icon} size={22} /></span><span>{n.label}</span>
				</a>
			{/each}
		</nav>
	{/if}
{/if}

<style>
	.splash {
		min-height: var(--app-h);
		display: grid;
		place-items: center;
	}
	.dot {
		width: 14px;
		height: 14px;
		border-radius: 50%;
		background: var(--accent);
		animation: pulse 1s ease-in-out infinite alternate;
	}
	@keyframes pulse {
		to {
			opacity: 0.2;
			transform: scale(0.7);
		}
	}
	.shell {
		display: grid;
		grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
		min-height: var(--app-h);
	}
	.side {
		position: sticky;
		top: 0;
		height: calc(var(--app-h) - var(--bar-h));
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		padding: 1.25rem 0.75rem;
		overflow-y: auto;
		border-right: 1px solid var(--line);
		background: var(--bg-raised);
	}
	.brand {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		font-weight: 700;
		padding: 0 0.6rem 1.1rem;
	}
	.logo {
		width: 28px;
		height: 28px;
		flex: none;
	}
	.side a {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		padding: 0.55rem 0.7rem;
		border-radius: var(--radius-sm);
		color: var(--fg-muted);
		font-weight: 550;
	}
	.side a:hover {
		background: var(--bg-sunken);
		color: var(--fg);
	}
	.side a.active {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	/* Material: a navigation rail with pill-shaped indicators and no dividing line. */
	:global(:root[data-theme='material']) .side {
		border-right-color: transparent;
		background: var(--bg);
	}
	:global(:root[data-theme='material']) .side a {
		border-radius: 999px;
		padding: 0.65rem 1rem;
	}
	.chip {
		display: flex;
		align-items: center;
		gap: 0.45rem;
		width: fit-content;
		max-width: calc(100% - 2 * clamp(1rem, 3vw, 2.5rem));
		margin: 0.6rem clamp(1rem, 3vw, 2.5rem) 0;
		padding: 0.3rem 0.75rem;
		border-radius: 999px;
		font-size: 0.85rem;
		background: var(--bg-sunken);
		color: var(--fg);
		border: 1px solid var(--line);
		flex: none;
	}
	.chip.busy {
		border-color: var(--accent);
		background: var(--accent-soft);
	}
	.content {
		min-width: 0;
		padding-bottom: calc(var(--bar-h) + var(--tabs-h) + 1rem);
	}
	/* Now playing's ambient background sits behind this area, inside it. */
	.content.on-np {
		position: relative;
		isolation: isolate;
	}
	/* Room for the "Moving playback…" strip above the player bar, while it shows. */
	.content.has-strip:not(.on-np) {
		padding-bottom: calc(var(--bar-h) + var(--tabs-h) + var(--strip-h) + 1rem);
	}
	/* Now playing has its own transport, seek, volume and Play on at every width: no second
	   one in the player bar under it (Wave 3.5). */
	.barwrap.on-np {
		display: none;
	}
	.content.on-np {
		padding-bottom: calc(var(--tabs-h) + 1rem);
	}
	.banner {
		margin: 1rem clamp(1rem, 3vw, 2.5rem) 0;
		padding: 0.6rem 0.9rem;
		border: 1px solid currentColor;
		border-radius: var(--radius-sm);
	}
	.resume {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
		border-color: var(--accent);
		background: var(--accent-soft);
	}
	.resume span {
		flex: 1 1 14rem;
	}
	.tap {
		width: calc(100% - 2 * clamp(1rem, 3vw, 2.5rem));
		font: inherit;
		color: inherit;
		text-align: left;
		cursor: pointer;
	}
	.tabs {
		display: none;
	}
	@media (max-width: 760px) {
		:global(:root) {
			/* Bottom chrome kept tight (tabs, mini player, "Playing on" strip): each control
			   still has a 44 px target. */
			--tabs-h: calc(52px + env(safe-area-inset-bottom));
			--bar-h: 58px;
			--strip-h: 30px;
		}
		.shell {
			grid-template-columns: minmax(0, 1fr);
		}
		.side {
			display: none;
		}
		.tabs {
			position: fixed;
			left: 0;
			right: 0;
			bottom: 0;
			height: var(--tabs-h);
			padding-bottom: env(safe-area-inset-bottom);
			display: grid;
			grid-template-columns: repeat(4, 1fr);
			background: var(--bg-raised);
			border-top: 1px solid var(--line);
			z-index: 21;
		}
		.tabs a {
			display: grid;
			justify-items: center;
			min-width: 0;
			align-content: center;
			gap: 2px;
			font-size: 0.7rem;
			color: var(--fg-muted);
		}
		.tabs a.active {
			color: var(--accent);
		}
		/* Every tab's icon sits in the same fixed slot, so labels line up and the active
		   tab's pill (Material) never shifts or resizes anything. */
		.tabs .ico {
			display: grid;
			place-items: center;
			width: 56px;
			height: 28px;
			border-radius: 999px;
		}
		/* Material: a navigation bar with a pill behind the active icon. */
		:global(:root[data-theme='material']) .tabs {
			border-top-color: transparent;
			background: var(--bg-sunken);
		}
		:global(:root[data-theme='material']) .tabs a.active {
			color: var(--fg);
		}
		:global(:root[data-theme='material']) .tabs a.active .ico {
			background: var(--accent-soft);
			color: var(--accent-soft-fg);
		}
	}
	/* Phones and narrow windows (Now playing's phone layout, up to 840 px): the page has its
	   own controls, so no mini bar there… */
	@media (max-width: 840px) {
		.barwrap.on-np {
			display: none;
		}
		/* …so no room is kept for it either: the page is exactly the visible height (100dvh)
		   and ends at the tab bar. Banners (a notice, "tap to turn the sound on") take their
		   room from Now playing, which fills the rest and never scrolls the page. */
		.content.on-np {
			display: flex;
			flex-direction: column;
			height: var(--app-h);
			padding-bottom: var(--tabs-h);
			overflow: hidden;
		}
	}
</style>
