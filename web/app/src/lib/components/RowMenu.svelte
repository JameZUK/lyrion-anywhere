<script lang="ts" module>
	export interface RowAction {
		label: string;
		icon: string;
		run: () => void;
		/** A key that does the same on the focused row, shown to screen readers. */
		shortcut?: string;
		/** Shown but unavailable (e.g. it needs the server): stays focusable, does nothing. */
		disabled?: boolean;
		/** Why it is unavailable, when the connection is the reason (title and description). */
		why?: { title?: string; 'aria-describedby'?: string };
		/** A destructive action: kept last, after a divider (Wave 3.5 rule 3). */
		danger?: boolean;
		/** A hook for tests and screenshots (`data-action`). */
		id?: string;
		/** One of a set of choices (a menuitemradio), chosen or not. */
		checked?: boolean;
	}
</script>

<script lang="ts">
	// A row's "More options" menu button (WAI-ARIA APG menu button): Enter, Space or the arrow
	// keys open it with focus on an item; arrows, Home and End move; Escape closes it and puts
	// focus back on the button; Tab or a click elsewhere closes it. `show()` opens it from
	// elsewhere (a long press or a right-click on the row).
	import Icon from './Icon.svelte';

	let {
		label,
		actions,
		icon = 'more',
		text = '',
		trigger: data = {},
	}: {
		label: string;
		actions: readonly RowAction[];
		/** The trigger's icon (⋯ by default). */
		icon?: string;
		/** A visible label beside the icon (header menus on wide screens). */
		text?: string;
		/** Extra attributes for the trigger (e.g. a `data-` hook). */
		trigger?: Record<string, string | boolean | undefined>;
	} = $props();
	/** Destructive actions last, after the rest. */
	const ordered = $derived([...actions.filter((a) => !a.danger), ...actions.filter((a) => a.danger)]);

	let open = $state(false);
	let menu = $state<HTMLDivElement>();
	let trigger = $state<HTMLButtonElement>();
	const uid = `rm-${Math.random().toString(36).slice(2, 9)}`;
	const items = () => [...(menu?.querySelectorAll<HTMLButtonElement>('[role^="menuitem"]') ?? [])];

	export function show(focus: 'first' | 'last' = 'first') {
		open = true;
		queueMicrotask(() => {
			const all = items();
			(focus === 'last' ? all.at(-1) : all[0])?.focus();
		});
	}

	function close(refocus = true) {
		open = false;
		if (refocus) trigger?.focus();
	}

	function triggerKey(e: KeyboardEvent) {
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			e.stopPropagation();
			show(e.key === 'ArrowUp' ? 'last' : 'first');
		}
	}

	function menuKey(e: KeyboardEvent) {
		const all = items();
		const at = all.indexOf(document.activeElement as HTMLButtonElement);
		let to = -1;
		if (e.key === 'ArrowDown') to = (at + 1) % all.length;
		else if (e.key === 'ArrowUp') to = (at - 1 + all.length) % all.length;
		else if (e.key === 'Home') to = 0;
		else if (e.key === 'End') to = all.length - 1;
		else if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			close();
			return;
		} else if (e.key === 'Tab') {
			close(false);
			return;
		}
		if (to >= 0) {
			e.preventDefault();
			e.stopPropagation();
			all[to]?.focus();
		}
	}

	function choose(a: RowAction) {
		if (a.disabled) return;
		close();
		a.run();
	}

	$effect(() => {
		if (!open) return;
		const outside = (e: PointerEvent) => {
			const t = e.target as Node;
			if (!menu?.contains(t) && !trigger?.contains(t)) close(false);
		};
		document.addEventListener('pointerdown', outside, true);
		return () => document.removeEventListener('pointerdown', outside, true);
	});
</script>

<div class="rowmenu">
	<button
		type="button"
		class="icon-btn"
		class:labelled={!!text}
		bind:this={trigger}
		id={uid}
		aria-label={label}
		aria-haspopup="menu"
		aria-expanded={open}
		aria-controls={open ? `${uid}-menu` : undefined}
		onclick={() => (open ? close() : show())}
		onkeydown={triggerKey}
		data-row-more
		{...data}
	>
		<Icon name={icon} size={20} />{#if text}<span class="text">{text}</span>{/if}
	</button>
	{#if open}
		<div class="menu" role="menu" id={`${uid}-menu`} aria-labelledby={uid} bind:this={menu} tabindex="-1" onkeydown={menuKey}>
			{#each ordered as a, i (a.label)}
				{#if a.danger && i > 0 && !ordered[i - 1]?.danger}<hr aria-hidden="true" />{/if}
				<button role={a.checked === undefined ? 'menuitem' : 'menuitemradio'} aria-checked={a.checked} tabindex="-1" class:danger={a.danger} onclick={() => choose(a)} aria-keyshortcuts={a.shortcut} aria-disabled={a.disabled ? 'true' : undefined} {...a.disabled ? a.why ?? {} : {}} data-action={a.id}>
					<Icon name={a.icon} size={18} /><span>{a.label}</span>{#if a.shortcut}<kbd class="muted" aria-hidden="true">{a.shortcut}</kbd>{/if}
				</button>
			{/each}
		</div>
	{/if}
</div>

<style>
	.rowmenu {
		position: relative;
		flex: none;
	}
	.menu {
		position: absolute;
		right: 0;
		top: 100%;
		z-index: 6;
		display: grid;
		min-width: 200px;
		padding: 0.35rem;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		box-shadow: var(--shadow);
	}
	hr {
		border: 0;
		border-top: 1px solid var(--line);
		margin: 0.25rem 0.4rem;
	}
	.menu button.danger {
		color: var(--danger);
	}
	.labelled {
		width: auto;
		display: inline-flex;
		gap: 0.4rem;
		padding: 0 0.9rem;
		border-radius: 999px;
	}
	.text {
		font-weight: 600;
	}
	.menu button[aria-disabled='true'] {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.menu button {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 44px;
		padding: 0.55rem 0.7rem;
		border: 0;
		background: transparent;
		border-radius: 6px;
		text-align: left;
		color: var(--fg);
	}
	.menu button span {
		flex: 1;
	}
	.menu button:hover,
	.menu button:focus-visible {
		background: var(--bg-sunken);
	}
	kbd {
		font: inherit;
		font-size: 0.75rem;
		border: 1px solid var(--line);
		border-radius: 4px;
		padding: 0 0.3rem;
	}
</style>
