<script lang="ts">
	// A track's credits (Wave 2 §3.1): every role LMS reports, each name its own link to that
	// contributor's albums in that role. Names LMS can't tie to an id are plain text.
	import { api } from '#lib/state/app.svelte';
	import * as cmd from '#lib/lms/commands';
	import { parseCredits, roleLink, type Credit } from '#lib/lms/credits';
	import type { TitlesResult } from '#lib/lms/types';

	let { track, onfollow }: { track: number; onfollow?: () => void } = $props();

	let credits = $state<Credit[] | null>(null);
	let failed = $state(false);
	const uid = `credits-${Math.random().toString(36).slice(2, 9)}`;

	$effect(() => {
		const id = track;
		credits = null;
		failed = false;
		let live = true;
		let c: ReturnType<typeof cmd.trackCredits>;
		try {
			c = cmd.trackCredits(id);
		} catch {
			failed = true;
			return;
		}
		api.rpc<TitlesResult>('', c).then(
			(r) => {
				if (live) credits = parseCredits(r?.titles_loop?.[0]);
			},
			() => {
				if (live) failed = true;
			},
		);
		return () => (live = false);
	});
</script>

{#if credits?.length}
	<section class="credits" aria-labelledby={uid} data-credits>
		<h3 id={uid}>Credits</h3>
		<dl>
			{#each credits as c (c.role)}
				<div class="row">
					<dt class="muted">{c.label}</dt>
					<dd>
						<ul>
							{#each c.people as p, i (i)}
								{@const href = p.id === null ? null : roleLink(p.id, c.role)}
								<li>
									{#if href}<a {href} onclick={() => onfollow?.()} aria-label={`${p.name}, ${c.label.toLowerCase()}`}>{p.name}</a>{:else}<span>{p.name}</span>{/if}
								</li>
							{/each}
						</ul>
					</dd>
				</div>
			{/each}
		</dl>
	</section>
{:else if failed}
	<p class="muted note">Credits aren’t available for this track.</p>
{/if}

<style>
	.credits {
		margin: 0 0 1rem;
	}
	h3 {
		margin: 0 0 0.4rem;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
	}
	dl {
		margin: 0;
		display: grid;
		gap: 0.25rem;
	}
	.row {
		display: grid;
		grid-template-columns: minmax(6.5rem, max-content) 1fr;
		gap: 0.75rem;
		align-items: baseline;
	}
	dt {
		font-size: 0.85rem;
	}
	dd {
		margin: 0;
		min-width: 0;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0 0.25rem;
	}
	li:not(:last-child)::after {
		content: ',';
	}
	a {
		display: inline-flex;
		align-items: center;
		min-height: 44px;
		font-weight: 600;
		text-decoration: underline;
		text-decoration-color: color-mix(in srgb, currentColor 35%, transparent);
		text-underline-offset: 0.2em;
	}
	.note {
		font-size: 0.85rem;
		margin: 0 0 0.75rem;
	}
</style>
