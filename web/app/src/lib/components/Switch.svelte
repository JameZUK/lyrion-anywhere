<script lang="ts">
	// The one on/off control (Wave 3.5): a row-wide button with role="switch", its label on the
	// left and the switch on the right, with the state in words ("On"/"Off") beside the switch,
	// never in colour alone. At least 48 px tall; a one-sentence `note` may sit under the label.
	import type { HTMLButtonAttributes } from 'svelte/elements';

	let {
		checked,
		onchange,
		label,
		note = '',
		disabled = false,
		icon,
		compact = false,
		...rest
	}: {
		checked: boolean;
		onchange: (on: boolean) => void;
		label: string;
		note?: string;
		disabled?: boolean;
		/** Something before the label (e.g. a player's icon). */
		icon?: import('svelte').Snippet;
		/** Just the switch and its state (in a row that already names it); the label is its name. */
		compact?: boolean;
	} & Omit<HTMLButtonAttributes, 'onchange' | 'role' | 'aria-checked'> = $props();
	const uid = $props.id();
</script>

<button
	type="button"
	class="switch"
	class:compact
	role="switch"
	aria-checked={checked}
	aria-label={label}
	aria-describedby={note ? `${uid}-note` : undefined}
	{disabled}
	onclick={() => onchange(!checked)}
	{...rest}
>
	{#if icon}<span class="icon">{@render icon()}</span>{/if}
	{#if !compact}
		<span class="words">
			<span class="label">{label}</span>
			{#if note}<span class="note muted" id={`${uid}-note`}>{note}</span>{/if}
		</span>
	{/if}
	<span class="state" aria-hidden="true">{checked ? 'On' : 'Off'}</span>
	<span class="track" class:on={checked} aria-hidden="true"><span class="knob"></span></span>
</button>

<style>
	.switch {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		min-height: 48px;
		padding: 0.4rem 0.5rem;
		border: 0;
		border-radius: var(--radius-sm);
		background: transparent;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.switch.compact {
		width: auto;
		flex: none;
	}
	.switch:hover:not(:disabled) {
		background: var(--bg-sunken);
	}
	.switch:disabled {
		opacity: var(--disabled-opacity, 0.5);
		cursor: default;
	}
	.icon {
		display: inline-grid;
		flex: none;
	}
	.words {
		flex: 1;
		min-width: 0;
		display: grid;
	}
	.label {
		font-weight: 550;
	}
	.note {
		font-size: 0.82rem;
	}
	.state {
		flex: none;
		font-size: 0.82rem;
		font-weight: 600;
		color: var(--fg-muted);
		min-width: 2.2em;
		text-align: right;
	}
	.track {
		flex: none;
		position: relative;
		width: 40px;
		height: 24px;
		border-radius: 12px;
		background: var(--line);
		transition: background 0.15s;
	}
	.track.on {
		background: var(--accent);
	}
	.knob {
		position: absolute;
		top: 3px;
		left: 3px;
		width: 18px;
		height: 18px;
		border-radius: 50%;
		background: var(--bg-raised);
		box-shadow: 0 1px 2px rgb(0 0 0 / 30%);
		transition: transform 0.15s;
	}
	.track.on .knob {
		transform: translateX(16px);
	}
	@media (prefers-reduced-motion: reduce) {
		.track,
		.knob {
			transition: none;
		}
	}
</style>
