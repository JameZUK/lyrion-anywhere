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

	// A radiogroup the keyboard can drive: one stop in the tab order (the chosen choice, else the
	// first available), the arrow keys move between the available choices (round at the ends),
	// Home and End go to the first and last, and Enter or Space chooses (which closes the picker,
	// so the arrows only move the focus).
	let group = $state<HTMLElement>();
	const focusable = $derived(options.find((o) => o.value === value && !o.disabled)?.value ?? options.find((o) => !o.disabled)?.value);
	$effect(() => {
		group?.querySelector<HTMLElement>('[role=radio][tabindex="0"]')?.focus();
	});
	function onkey(e: KeyboardEvent) {
		if (!group || !['ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft', 'Home', 'End'].includes(e.key)) return;
		const radios = [...group.querySelectorAll<HTMLButtonElement>('[role=radio]:not(:disabled)')];
		if (!radios.length) return;
		e.preventDefault();
		const n = radios.length;
		const at = radios.indexOf(document.activeElement as HTMLButtonElement);
		const forward = e.key === 'ArrowDown' || e.key === 'ArrowRight';
		const next = e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : at < 0 ? (forward ? 0 : n - 1) : (at + (forward ? 1 : n - 1)) % n;
		radios[next]!.focus();
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
		<div class="choices" role="radiogroup" aria-label={label} data-picker={id} bind:this={group} onkeydown={onkey} tabindex="-1">
			{#each options as o (o.value)}
				<button
					class="choice"
					type="button"
					role="radio"
					aria-checked={o.value === value}
					disabled={o.disabled}
					tabindex={o.value === focusable ? 0 : -1}
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
