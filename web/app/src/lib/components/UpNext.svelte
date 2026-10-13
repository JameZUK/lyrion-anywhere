<script lang="ts">
	// "Up next" under the transport controls: the next track, with its cover. Tap opens the
	// queue; swipe the row sideways (or press Delete, or the remove button) to take that
	// track out of the queue, with Undo.
	import { api, app } from '$lib/state/app.svelte';
	import * as cmd from '$lib/lms/commands';
	import { textOf } from '$lib/lms/format';
	import { isSwipe, removeChecked, restoreTrack } from '$lib/lms/queue-edit';
	import { upNext } from '$lib/lms/upnext';
	import type { Track } from '$lib/lms/types';
	import Art from './Art.svelte';
	import Icon from './Icon.svelte';
	import Snackbar from './Snackbar.svelte';

	/** A swipe must travel this far (px) to remove. */
	const SWIPE_REMOVE_PX = 90;

	const next = $derived(upNext(app.now));
	/** Hidden at once when removed, until the next status says what follows now. */
	let removed = $state<string | null>(null);
	let undo = $state<{ track: Track; index: number; player: string } | null>(null);
	let notice = $state('');
	let swipe = $state<{ id: number; x: number; y: number; dx: number; active: boolean } | null>(null);
	let swallowClick = false;

	const key = (t: Track | undefined, i: number) => `${app.playerId}|${i}|${t?.id ?? t?.title ?? ''}`;
	const shown = $derived(next && key(next.track, next.index) !== removed ? next : null);
	const title = $derived(textOf(next?.track.title) || 'Next track');
	const artist = $derived(textOf(next?.track.artist));

	async function remove() {
		const n = shown;
		const player = app.playerId;
		if (!n || !player) return;
		removed = key(n.track, n.index);
		undo = null;
		notice = '';
		try {
			await removeChecked((p, c) => api.rpc(p, c), player, n.index, n.track);
			const canUndo = Number.isSafeInteger(n.track.id) && (n.track.id as number) > 0;
			undo = canUndo ? { track: n.track, index: n.index, player } : null;
			notice = `Removed ${textOf(n.track.title, 80) || 'track'}`;
		} catch (e) {
			removed = null;
			notice = `Couldn't remove the track: ${(e as Error).message}`;
		}
		app.queueVersion++;
		app.soon();
	}

	async function restore() {
		const u = undo;
		dismiss();
		if (!u || u.player !== app.playerId) return;
		try {
			await restoreTrack((p, c) => api.rpc(p, c), u.player, u.track.id as number, u.index);
		} catch (e) {
			notice = `Couldn't put the track back: ${(e as Error).message}`;
		}
		removed = null;
		app.queueVersion++;
		app.soon();
	}

	function dismiss() {
		undo = null;
		notice = '';
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Delete' || e.key === 'Backspace') {
			e.preventDefault();
			void remove();
		}
	}

	// --- swipe sideways to remove (touch and pen; vertical movement stays a scroll) ---------

	function down(e: PointerEvent) {
		if (e.pointerType === 'mouse' || swipe) return;
		swipe = { id: e.pointerId, x: e.clientX, y: e.clientY, dx: 0, active: false };
	}
	function move(e: PointerEvent) {
		if (!swipe || e.pointerId !== swipe.id) return;
		const dx = e.clientX - swipe.x;
		const dy = e.clientY - swipe.y;
		if (!swipe.active) {
			if (!isSwipe(dx, dy)) {
				if (Math.abs(dy) > 12) swipe = null; // a scroll: not ours
				return;
			}
			(e.currentTarget as Element).setPointerCapture?.(e.pointerId);
		}
		swipe = { ...swipe, dx, active: true };
	}
	function up(e: PointerEvent, cancelled = false) {
		if (!swipe || e.pointerId !== swipe.id) return;
		const { dx, active } = swipe;
		swipe = null;
		if (!active) return;
		swallowClick = true;
		if (!cancelled && Math.abs(dx) >= SWIPE_REMOVE_PX) void remove();
	}
	function onclick(e: MouseEvent) {
		if (swallowClick) {
			swallowClick = false;
			e.preventDefault();
		}
	}
</script>

{#if shown}
	<div class="upnext" data-up-next>
		<a
			class="row"
			href="#/queue"
			aria-label={`Up next: ${title}${artist ? ` by ${artist}` : ''}. Opens the queue; press Delete to remove it from the queue.`}
			style:transform={swipe?.active ? `translateX(${swipe.dx}px)` : undefined}
			style:opacity={swipe?.active ? Math.max(0.3, 1 - Math.abs(swipe.dx) / 240) : undefined}
			{onclick}
			{onkeydown}
			onpointerdown={down}
			onpointermove={move}
			onpointerup={(e) => up(e)}
			onpointercancel={(e) => up(e, true)}
		>
			<div class="thumb"><Art path={cmd.coverPath(shown.track.coverid ?? shown.track.artwork_track_id, 120)} alt="" label={title} /></div>
			<div class="meta">
				<span class="kicker">Up next</span>
				<span class="line ellipsis">{title}{#if artist}<span class="muted">{` — ${artist}`}</span>{/if}</span>
			</div>
		</a>
		<button class="icon-btn rm" aria-label={`Remove ${title} from the queue`} onclick={remove}><Icon name="close" size={18} /></button>
	</div>
{/if}
{#if notice}
	<Snackbar message={notice} action={undo ? 'Undo' : ''} onaction={restore} ondismiss={dismiss} />
{/if}

<style>
	.upnext {
		display: flex;
		align-items: center;
		gap: 0.25rem;
		max-width: 400px;
		width: 100%;
		margin-top: 0.25rem;
		padding: 0.3rem;
		border-radius: var(--radius-sm);
		background: color-mix(in srgb, var(--bg-raised) 55%, transparent);
	}
	.row {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.65rem;
		text-align: left;
		touch-action: pan-y;
		user-select: none;
		border-radius: var(--radius-sm);
	}
	.thumb {
		width: 40px;
		flex: none;
	}
	.meta {
		display: grid;
		min-width: 0;
	}
	.kicker {
		font-size: 0.72rem;
		font-weight: 650;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--fg-muted);
	}
	.line {
		font-size: 0.9rem;
	}
	.ellipsis {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.rm {
		flex: none;
		width: 36px;
		height: 36px;
		color: var(--fg-muted);
	}
	@media (prefers-reduced-motion: no-preference) {
		.upnext {
			animation: rise 0.3s ease-out;
		}
	}
	@keyframes rise {
		from {
			opacity: 0;
			transform: translateY(4px);
		}
	}
</style>
