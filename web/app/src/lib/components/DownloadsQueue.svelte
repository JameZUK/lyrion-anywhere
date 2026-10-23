<script lang="ts">
	// The downloads queue in the main Queue (Wave 3.5 item 14): what this device plays from its
	// downloads, in play order, the current track marked. A tap plays from that track. It isn't
	// edited here: the downloads engine has no moves or removals (Downloads holds the content).
	import { duration, textOf } from '#lib/lms/format';
	import { controls } from '#lib/player/controls.svelte';
	import { now } from '#lib/player/now.svelte';
	import Icon from './Icon.svelte';

	let { onplay }: { onplay?: () => void } = $props();

	function play(i: number) {
		controls.jump(i);
		onplay?.();
	}
</script>

<ol class="dq" aria-label="Downloads queue" data-downloads-queue>
	{#each now.queue as t, i (`${i}:${t.id}`)}
		<li class:now={i === now.index}>
			<button onclick={() => play(i)} aria-current={i === now.index ? 'true' : undefined} aria-label={`Play ${textOf(t.title)}${t.artist ? ` by ${textOf(t.artist)}` : ''}. Downloaded`}>
				<span class="num muted">{i + 1}</span>
				<span class="text">
					<span class="title ellipsis">{textOf(t.title)}</span>
					<span class="sub ellipsis muted">{[textOf(t.artist), textOf(t.album)].filter(Boolean).join(' · ')}</span>
				</span>
				<span class="mark" aria-hidden="true"><Icon name="downloaded" size={15} /></span>
				<span class="dur muted">{duration(t.duration)}</span>
			</button>
		</li>
	{/each}
</ol>

<style>
	.dq {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	li {
		border-radius: var(--radius-sm);
	}
	li.now {
		background: var(--accent-soft);
	}
	li.now .title {
		color: var(--accent);
	}
	button {
		display: flex;
		align-items: center;
		gap: 0.8rem;
		width: 100%;
		min-height: 52px;
		padding: 0.45rem 0.6rem;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	button:hover {
		background: var(--bg-sunken);
	}
	.num {
		width: 2ch;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.text {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.title {
		font-weight: 550;
	}
	.sub,
	.dur {
		font-size: 0.85rem;
	}
	.mark {
		display: inline-grid;
		color: var(--accent);
	}
	.ellipsis {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
</style>
