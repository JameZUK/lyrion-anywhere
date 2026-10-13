<script lang="ts" generics="T extends string">
	// Filter chips as tabs (WAI-ARIA APG, automatic activation): one tab stop; the arrow keys,
	// Home and End move between chips and choose; the chosen chip controls `panel`.
	let {
		items,
		value,
		onchange,
		label,
		panel,
		idPrefix = `chips-${Math.random().toString(36).slice(2, 9)}`,
	}: {
		items: readonly { value: T; label: string }[];
		value: T;
		onchange: (v: T) => void;
		/** What the chips choose, for screen readers ("Show"). */
		label: string;
		/** Id of the panel the chips control. */
		panel?: string;
		idPrefix?: string;
	} = $props();

	let list = $state<HTMLDivElement>();

	function onkeydown(e: KeyboardEvent) {
		const at = items.findIndex((i) => i.value === value);
		let to = -1;
		if (e.key === 'ArrowRight' || e.key === 'ArrowDown') to = (at + 1) % items.length;
		else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') to = (at - 1 + items.length) % items.length;
		else if (e.key === 'Home') to = 0;
		else if (e.key === 'End') to = items.length - 1;
		const it = items[to];
		if (!it) return;
		e.preventDefault();
		onchange(it.value);
		queueMicrotask(() => list?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[to]?.focus());
	}
</script>

<div class="chips" role="tablist" aria-label={label} bind:this={list} {onkeydown} tabindex="-1">
	{#each items as it (it.value)}
		<button
			type="button"
			role="tab"
			id={`${idPrefix}-${it.value}`}
			aria-selected={it.value === value}
			aria-controls={panel}
			tabindex={it.value === value ? 0 : -1}
			class:on={it.value === value}
			onclick={() => onchange(it.value)}
			data-chip={it.value}
		>
			{it.label}
		</button>
	{/each}
</div>

<style>
	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		margin: 0.5rem 0;
		outline: none;
	}
	button {
		min-height: 44px;
		border: 1px solid var(--line);
		background: var(--bg-raised);
		color: var(--fg-muted);
		font: inherit;
		font-weight: 600;
		font-size: 0.9rem;
		padding: 0 1rem;
		border-radius: 999px;
		cursor: pointer;
	}
	button.on {
		background: var(--accent-soft);
		border-color: transparent;
		color: var(--accent-soft-fg);
	}
</style>
