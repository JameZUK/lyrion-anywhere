<script lang="ts">
	// Transport for the offline player: what is playing from this device's downloads.
	import { offlinePlayer as p } from '$lib/offline/index.svelte';
	import { duration } from '$lib/lms/format';
	import Icon from './Icon.svelte';

	let { href = '#/downloads' }: { href?: string | null } = $props();

	let seeking = $state<number | null>(null);
	const t = $derived(p.current);
	const shown = $derived(seeking ?? p.position);
	const sub = $derived([t?.artist, t?.album].filter(Boolean).join(' · '));
</script>

<footer class="bar" aria-label="Offline player">
	<svelte:element this={href ? 'a' : 'div'} class="now" href={href ?? undefined}>
		<div class="thumb">
			{#if p.cover}<img src={p.cover} alt="" />{:else}<span class="ph" aria-hidden="true">♪</span>{/if}
		</div>
		<div class="meta">
			<div class="title ellipsis">{t?.title ?? ''}</div>
			<div class="sub ellipsis muted"><Icon name="download" size={12} /> {p.error || sub}</div>
		</div>
	</svelte:element>

	<div class="centre">
		<div class="transport">
			<button class="icon-btn" aria-label="Previous" onclick={() => p.previous()}><Icon name="prev" /></button>
			<button class="icon-btn play" aria-label={p.playing ? 'Pause' : 'Play'} onclick={() => p.toggle()}>
				<Icon name={p.playing ? 'pause' : 'play'} size={24} />
			</button>
			<button class="icon-btn" aria-label="Next" onclick={() => p.next()} disabled={p.index >= p.queue.length - 1}><Icon name="next" /></button>
		</div>
		<div class="progress">
			<span class="time">{duration(shown)}</span>
			<input
				type="range"
				min="0"
				max={Math.max(p.duration, 1)}
				step="1"
				value={shown}
				aria-label="Position"
				oninput={(e) => (seeking = Number(e.currentTarget.value))}
				onchange={(e) => {
					p.seek(Number(e.currentTarget.value));
					seeking = null;
				}}
			/>
			<span class="time">{duration(p.duration)}</span>
		</div>
	</div>

	<div class="right">
		<span class="tag muted">Downloads</span>
		<button class="icon-btn" aria-label="Stop playing downloads" onclick={() => p.stop()}><Icon name="close" size={20} /></button>
	</div>
</footer>

<style>
	.bar {
		position: fixed;
		left: 0;
		right: 0;
		bottom: var(--tabs-h, 0px);
		height: var(--bar-h);
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 2fr) minmax(0, 1fr);
		align-items: center;
		gap: 1rem;
		padding: 0 1rem;
		background: color-mix(in srgb, var(--bg-raised) 92%, transparent);
		backdrop-filter: blur(14px);
		border-top: 1px solid var(--line);
		z-index: 20;
	}
	.now {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-width: 0;
	}
	.thumb {
		position: relative;
		width: 52px;
		aspect-ratio: 1;
		flex: none;
		overflow: hidden;
		border-radius: var(--radius-sm);
		background: linear-gradient(135deg, var(--bg-sunken), var(--line));
	}
	.thumb img {
		width: 100%;
		height: 100%;
		object-fit: cover;
	}
	.ph {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		color: var(--fg-muted);
	}
	.meta {
		min-width: 0;
	}
	.title {
		font-weight: 600;
	}
	.sub {
		font-size: 0.85rem;
	}
	.centre {
		display: grid;
		justify-items: center;
		gap: 0.1rem;
	}
	.transport {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	.play {
		background: var(--fg);
		color: var(--bg);
		width: 44px;
		height: 44px;
	}
	.play:hover {
		background: var(--fg);
	}
	.progress {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		width: min(100%, 560px);
	}
	.progress input {
		flex: 1;
	}
	.time {
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
		min-width: 3.2em;
		text-align: center;
	}
	.right {
		display: flex;
		align-items: center;
		justify-content: flex-end;
		gap: 0.5rem;
	}
	.tag {
		font-size: 0.8rem;
	}
	@media (max-width: 760px) {
		.bar {
			grid-template-columns: minmax(0, 1fr) auto auto;
			gap: 0.4rem;
			padding: 0 0.6rem;
		}
		.thumb {
			width: 44px;
		}
		.progress,
		.tag,
		.transport .icon-btn:first-child {
			display: none;
		}
		.play {
			width: 40px;
			height: 40px;
		}
	}
</style>
