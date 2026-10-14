<script lang="ts" generics="T extends string">
	// A setting with a list of choices (Wave 3.5, rule 5): one row with its label and the value
	// chosen; a tap opens a picker (a sheet of radio choices) instead of a list laid out inline.
	// The one shared component for every such setting. A choice may be unavailable, saying why.
	import Icon from './Icon.svelte';
	import Sheet from './Sheet.svelte';

	let {
		label,
		value,
		options,
		onchange,
		note = '',
		disabled = false,
		id,
	}: {
		label: string;
		value: T;
		options: readonly { value: T; label: string; note?: string; disabled?: boolean }[];
		onchange: (v: T) => void;
		/** One sentence of help, under the label. */
		note?: string;
		disabled?: boolean;
		/** A hook for tests and screenshots (`data-setting`). */
		id?: string;
	} = $props();

	const uid = $props.id();
	let open = $state(false);
	const current = $derived(options.find((o) => o.value === value));

	function choose(v: T) {
		open = false;
		if (v !== value) onchange(v);
	}
</script>

<button class="picker-row" type="button" aria-haspopup="dialog" aria-describedby={note ? `${uid}-note` : undefined} {disabled} onclick={() => (open = true)} data-setting={id}>
	<span class="words">
		<span class="label">{label}</span>
		{#if note}<span class="note muted" id={`${uid}-note`}>{note}</span>{/if}
	</span>
	<span class="value" data-setting-value>{current?.label ?? value}</span>
	<span class="chev" aria-hidden="true">›</span>
</button>
{#if open}
	<Sheet title={label} onclose={() => (open = false)}>
		<div class="choices" role="radiogroup" aria-label={label} data-picker={id}>
			{#each options as o (o.value)}
				<button
					class="choice"
					type="button"
					role="radio"
					aria-checked={o.value === value}
					disabled={o.disabled}
					onclick={() => choose(o.value)}
					data-choice={o.value}
				>
					<span class="tick" aria-hidden="true">{#if o.value === value}<Icon name="check" size={18} />{/if}</span>
					<span class="words"><span>{o.label}</span>{#if o.note}<small class="muted">{o.note}</small>{/if}</span>
				</button>
			{/each}
		</div>
	</Sheet>
{/if}

<style>
	.picker-row,
	.choice {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		min-height: 52px;
		padding: 0.5rem;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.picker-row:hover:not(:disabled),
	.choice:hover:not(:disabled) {
		background: var(--bg-sunken);
	}
	.picker-row:disabled,
	.choice:disabled {
		opacity: var(--disabled-opacity, 0.5);
		cursor: default;
	}
	.words {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.label {
		font-weight: 550;
	}
	.note,
	small {
		font-size: 0.82rem;
	}
	.value {
		flex: none;
		max-width: 50%;
		color: var(--fg-muted);
		text-align: right;
		overflow-wrap: anywhere;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.2rem;
	}
	.choices {
		display: grid;
		gap: 2px;
	}
	.choice[aria-checked='true'] {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.tick {
		display: inline-grid;
		place-items: center;
		width: 1.25rem;
		flex: none;
		color: var(--accent);
	}
</style>
