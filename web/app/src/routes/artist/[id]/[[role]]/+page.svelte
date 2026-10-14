<script lang="ts">
	// An artist: their albums, and "Appears as" chips for each role they have in the library
	// (Wave 2 §3.1). With a role (`#/artist/7/composer`), only the albums where they have that
	// role, with the role in the title ("Name · Composer").
	import BackButton from '$lib/components/BackButton.svelte';
	import { useDetailTitle } from '$lib/title.svelte';
	import { page } from '$app/state';
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { CREDIT_ORDER, ROLE_LABELS, nameInRole, roleFromSlug, roleLink } from '$lib/lms/credits';
	import type { Album, AlbumsResult, TitlesResult } from '$lib/lms/types';
	import AlbumGrid from '$lib/components/AlbumGrid.svelte';
	import ContextMenu from '$lib/components/ContextMenu.svelte';
	import InfoText from '$lib/components/InfoText.svelte';
	import ArtistDownload from '$lib/components/ArtistDownload.svelte';
	import PageActions from '$lib/components/PageActions.svelte';
	import { NO_ARTIST_INFO, sanitiseArtistInfo, type ArtistInfo } from '$lib/info/info';

	let showMore = $state(false);
	let dl = $state<ReturnType<typeof ArtistDownload>>();

	let albums = $state<Album[]>([]);
	let name = $state('');
	let error = $state('');
	/** Albums per role, for the "Appears as" chips (roles with none left out). */
	let roles = $state<{ role: cmd.Role; count: number }[]>([]);

	const artistId = $derived.by(() => {
		try {
			return cmd.id(page.params.id);
		} catch {
			return null;
		}
	});
	/** The role in the address: undefined for none, null for one that isn't a role. */
	const role = $derived(page.params.role === undefined ? undefined : roleFromSlug(page.params.role));

	/** LMS lists the album artist; the most common one names the page. */
	function mostCommonArtist(list: Album[]): string {
		const counts = new Map<string, number>();
		for (const a of list) if (a.artist) counts.set(a.artist, (counts.get(a.artist) ?? 0) + 1);
		return [...counts.entries()].sort((x, y) => y[1] - x[1])[0]?.[0] ?? '';
	}

	$effect(() => {
		const id = artistId;
		const r = role;
		albums = [];
		name = '';
		error = id === null ? 'Not an artist.' : r === null ? 'Not a role.' : '';
		if (id === null || r === null) return;
		let live = true;
		api.rpc<AlbumsResult>('', cmd.artistAlbums(id, r)).then(
			(res) => {
				if (!live) return;
				albums = res.albums_loop ?? [];
				if (!r) name = mostCommonArtist(albums) || 'Artist';
			},
			(e) => live && (error = (e as Error).message),
		);
		// In a role, the albums' artist is often someone else: ask for the name as LMS credits it.
		if (r) {
			api.rpc<TitlesResult>('', cmd.roleTrack(id, r)).then(
				(res) => {
					if (live) name = nameInRole(res, id, r) || 'Artist';
				},
				() => live && (name = 'Artist'),
			);
		}
		return () => (live = false);
	});

	// "Appears as": one-row album queries per role; the answer's count is all that is used.
	$effect(() => {
		const id = artistId;
		void app.libraryVersion;
		roles = [];
		if (id === null) return;
		let live = true;
		const found = new Map<cmd.Role, number>();
		for (const r of CREDIT_ORDER) {
			api.rpc<AlbumsResult>('', cmd.roleAlbumCount(id, r)).then(
				(res) => {
					const n = Number(res?.count);
					if (!live || !Number.isSafeInteger(n) || n <= 0) return;
					found.set(r, n);
					roles = CREDIT_ORDER.filter((x) => found.has(x)).map((x) => ({ role: x, count: found.get(x)! }));
				},
				() => {},
			);
		}
		return () => (live = false);
	});

	// Biography and photo (Music & Artist Information plugin, through the gateway): loaded
	// lazily on the artist's own page; anything missing or failing is simply not shown.
	let info = $state<ArtistInfo>(NO_ARTIST_INFO);
	let photoSrc = $state('');
	$effect(() => {
		const id = artistId;
		const r = role;
		info = NO_ARTIST_INFO;
		if (id === null || r !== undefined) return;
		let live = true;
		api.info?.('artist', id).then(
			(v) => live && (info = sanitiseArtistInfo(v, 400)),
			() => {},
		);
		return () => (live = false);
	});
	$effect(() => {
		const p = info.photo;
		photoSrc = '';
		if (!p) return;
		let live = true;
		const release = api.retainArt?.(p);
		api.art(p).then(
			(u) => live && (photoSrc = u),
			() => {},
		);
		return () => {
			live = false;
			release?.();
		};
	});

	/** Chips are worth showing once there is more than the usual album artist / artist pair. */
	const showRoles = $derived(!!role || roles.some((r) => r.role !== 'ALBUMARTIST' && r.role !== 'ARTIST'));
	const roleLabel = $derived(role ? ROLE_LABELS[role] : '');
	useDetailTitle(() => (name ? (roleLabel ? `${textOf(name)} · ${roleLabel}` : textOf(name)) : ''));
</script>

<section class="page">
	<BackButton parent="/library" />
	{#if error}
		<p class="error">{error}</p>
	{:else if artistId !== null}
		{@const id = artistId}
		<div class="head" class:with-photo={!!photoSrc}>
			{#if photoSrc}
				<figure class="photo" data-artist-photo>
					<img src={photoSrc} alt={textOf(name) ? `Photo of ${textOf(name)}` : 'Artist photo'} draggable="false" onerror={() => (photoSrc = '')} />
					{#if info.credits}<figcaption class="muted">Photo: {info.credits}</figcaption>{/if}
				</figure>
			{/if}
			<div>
				<p class="kind muted">{roleLabel || 'Artist'}</p>
				<h1>{textOf(name) || '…'}{#if roleLabel}<span class="sr-only">{` · ${roleLabel}`}</span>{/if}</h1>
			</div>
		</div>
		{#if showRoles && roles.length}
			<nav class="roles" aria-label="Appears as" data-appears-as>
				<span class="muted lead" aria-hidden="true">Appears as</span>
				<ul>
					{#if role}
						<li><a class="chip" href={`#/artist/${id}`}>All albums</a></li>
					{/if}
					{#each roles as r (r.role)}
						<li>
							<a
								class="chip"
								class:on={role === r.role}
								href={roleLink(id, r.role)}
								aria-current={role === r.role ? 'page' : undefined}
								aria-label={`${ROLE_LABELS[r.role]}: ${r.count} ${r.count === 1 ? 'album' : 'albums'}`}
							>
								{ROLE_LABELS[r.role]} ({r.count.toLocaleString('en-GB')})
							</a>
						</li>
					{/each}
				</ul>
			</nav>
		{/if}
		{#if !role}
			<!-- Play, Shuffle, Download all albums and ⋯ (Add to queue, Download all as…, More…, Remove downloads). -->
			<PageActions sel={{ artist: id }} title={textOf(name)} can={albums.length > 0} next={false} onmore={() => (showMore = true)} extra={dl?.actions() ?? []}>
				{#snippet download()}<ArtistDownload bind:this={dl} {albums} />{/snippet}
			</PageActions>
		{:else}
			<div class="role-dl"><ArtistDownload {albums} /></div>
		{/if}
		{#if !role && info.biography}
			<InfoText heading="Biography" text={info.biography} />
		{/if}
		<h2>{albums.length} {albums.length === 1 ? 'album' : 'albums'}{roleLabel ? ` as ${roleLabel.toLowerCase()}` : ''}</h2>
		<AlbumGrid {albums} showArtist={!!role} />
		{#if showMore}<ContextMenu kind="artist" {id} title={textOf(name)} onclose={() => (showMore = false)} />{/if}
	{/if}
</section>

<style>
	.kind {
		text-transform: uppercase;
		letter-spacing: 0.08em;
		font-size: 0.75rem;
		font-weight: 650;
		margin: 0;
	}
	h1 {
		font-size: clamp(1.8rem, 3.5vw, 2.8rem);
		margin: 0.2rem 0 0.8rem;
	}
	.role-dl {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
		margin: 0.75rem 0;
	}
	.head + :global(.page-actions),
	.roles + :global(.page-actions) {
		margin-bottom: 1.5rem;
	}
	.head.with-photo {
		display: flex;
		align-items: center;
		gap: 1.25rem;
		margin-bottom: 0.8rem;
	}
	.photo {
		margin: 0;
		flex: none;
		width: clamp(96px, 18vw, 160px);
	}
	.photo img {
		display: block;
		width: 100%;
		aspect-ratio: 1;
		object-fit: cover;
		border-radius: 50%;
		box-shadow: var(--shadow);
	}
	.photo figcaption {
		font-size: 0.7rem;
		text-align: center;
		margin-top: 0.3rem;
	}
	h2 {
		font-size: 1rem;
		color: var(--fg-muted);
	}
	.roles {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.4rem 0.6rem;
		margin: 0 0 1.25rem;
	}
	.lead {
		font-size: 0.85rem;
		font-weight: 600;
	}
	.roles ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}
	.chip {
		display: inline-flex;
		align-items: center;
		min-height: 44px;
		box-sizing: border-box;
		padding: 0 0.9rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
		color: var(--fg-muted);
		font-weight: 600;
		font-size: 0.9rem;
	}
	.chip.on {
		background: var(--accent-soft);
		border-color: transparent;
		color: var(--accent-soft-fg);
	}
</style>
