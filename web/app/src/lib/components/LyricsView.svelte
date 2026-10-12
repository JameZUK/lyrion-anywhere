<script lang="ts">
	// Full-screen lyrics (docs/plans/wave1-design.md §9). Synced lyrics highlight the current
	// line and keep it in view (unless the listener is scrolling themselves); tapping a line
	// seeks there. Plain lyrics just scroll. Escape or Close returns to Now playing.
	import { app } from '$lib/state/app.svelte';
	import { lyrics } from '$lib/state/lyrics';
	import { lineAt } from '$lib/lyrics/lyrics';
	import { reducedMotion } from '$lib/theme/motion';
	import Icon from './Icon.svelte';

	let { title = '', onclose }: { title?: string; onclose: () => void } = $props();

	let dialog = $state<HTMLDialogElement>();
	let list = $state<HTMLElement>();
	/** The listener scrolled lately: leave the scroll position alone for a while. */
	let manualUntil = 0;
	const MANUAL_MS = 4000;

	const l = $derived(lyrics.lyrics);
	const current = $derived(lineAt(l, app.elapsed * 1000));
	const canSeek = $derived(!!l?.synced && app.now?.can_seek !== 0 && !!app.now);

	$effect(() => {
		const d = dialog;
		if (d && !d.open) d.showModal?.();
	});

	$effect(() => {
		const i = current;
		if (i < 0 || !list || performance.now() < manualUntil) return;
		const el = list.querySelector<HTMLElement>(`[data-line="${i}"]`);
		el?.scrollIntoView?.({ block: 'center', behavior: reducedMotion() ? 'auto' : 'smooth' });
	});

	function manual() {
		manualUntil = performance.now() + MANUAL_MS;
	}

	async function seekTo(ms: number) {
		manualUntil = 0;
		await app.seek(ms / 1000);
	}
</script>

<dialog bind:this={dialog} {onclose} class="lyrics" aria-label={title ? `Lyrics: ${title}` : 'Lyrics'} data-lyrics-view>
	<header>
		<h2>{title || 'Lyrics'}</h2>
		<button class="icon-btn" aria-label="Close lyrics" onclick={() => (dialog?.open ? dialog.close() : onclose())}><Icon name="close" /></button>
	</header>
	<div class="list" bind:this={list} onwheel={manual} ontouchmove={manual} onkeydown={manual} role="presentation">
		{#if lyrics.loading}
			<p class="muted">Looking for lyrics…</p>
		{:else if !l}
			<p class="muted">No lyrics for this track.</p>
		{:else if l.synced}
			<ol>
				{#each l.lines as line, i (i)}
					<li>
						<button
							class="line"
							class:now={i === current}
							class:past={i < current}
							aria-current={i === current ? 'true' : undefined}
							data-line={i}
							disabled={!canSeek}
							onclick={() => seekTo(line.t)}
						>{line.text || '♪'}</button>
					</li>
				{/each}
			</ol>
		{:else}
			<div class="plain">
				{#each l.lines as line, i (i)}<p data-line={i}>{line.text || ' '}</p>{/each}
			</div>
		{/if}
		{#if l}<p class="muted small source">{l.source === 'plugin' ? 'From Music & Artist Information' : 'From the file’s tags'}</p>{/if}
	</div>
</dialog>

<style>
	dialog.lyrics {
		border: 0;
		padding: 0;
		margin: 0;
		width: 100vw;
		height: var(--app-h, 100dvh);
		max-width: none;
		max-height: none;
		background: var(--bg);
		color: var(--fg);
		display: flex;
		flex-direction: column;
	}
	dialog.lyrics:not([open]) {
		display: none;
	}
	dialog::backdrop {
		background: var(--bg);
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		padding: 0.75rem 1rem;
		border-bottom: 1px solid var(--line);
	}
	h2 {
		margin: 0;
		font-size: 1.05rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.list {
		flex: 1;
		overflow-y: auto;
		padding: 1.5rem 1rem 40vh;
		max-width: 46rem;
		width: 100%;
		margin: 0 auto;
		box-sizing: border-box;
	}
	ol {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.line {
		display: block;
		width: 100%;
		text-align: left;
		background: none;
		border: 0;
		padding: 0.35rem 0.25rem;
		border-radius: var(--radius-sm);
		font: inherit;
		font-size: 1.5rem;
		line-height: 1.35;
		font-weight: 600;
		color: var(--fg-muted);
		cursor: pointer;
		overflow-wrap: anywhere;
	}
	.line:disabled {
		cursor: default;
	}
	.line.past {
		opacity: 0.75;
	}
	.line.now {
		color: var(--fg);
	}
	.line:focus-visible {
		outline: 2px solid var(--accent);
	}
	.plain p {
		margin: 0;
		font-size: 1.15rem;
		line-height: 1.6;
		overflow-wrap: anywhere;
	}
	.source {
		margin-top: 2rem;
	}
	@media (prefers-reduced-motion: no-preference) {
		.line {
			transition: color 0.25s;
		}
	}
</style>
