<script lang="ts">
	// Save the queue as a playlist: a name (checked as it's typed, by the gateway's rules),
	// then Save. A playlist of that name is never overwritten silently: the dialog says so
	// and offers Replace. A modal <dialog>: Escape or Cancel closes it, and focus goes back
	// to what opened it.
	import { me } from '#lib/state/me';
	import { ApiError } from '#lib/api/http';
	import type { SavedPlaylist } from '#lib/api/me';
	import { PLAYLIST_NAME_MAX, cleanPlaylistName, playlistNameProblem } from '#lib/queue/save-name';

	let {
		player,
		suggestion = '',
		onsaved,
		oncancel,
	}: {
		player: string;
		suggestion?: string;
		onsaved: (p: SavedPlaylist) => void;
		oncancel: () => void;
	} = $props();

	// The suggestion is only where the name starts.
	const start = (() => suggestion)();
	let name = $state(start);
	let error = $state('');
	let busy = $state(false);
	/** The playlist already saved under this name, while asking whether to replace it. */
	let exists = $state<SavedPlaylist | null>(null);
	/** Only say what's wrong with the name once something has been typed or Save pressed. */
	let touched = $state(false);

	let dialog = $state<HTMLDialogElement>();
	let input = $state<HTMLInputElement>();
	const id = `saveq-${Math.random().toString(36).slice(2, 9)}`;
	const opener = typeof document !== 'undefined' ? (document.activeElement as HTMLElement | null) : null;
	let finished = false;

	const problem = $derived(playlistNameProblem(cleanPlaylistName(name)));

	$effect(() => {
		const d = dialog;
		if (d && !d.open) d.showModal?.();
		input?.focus();
		input?.select();
	});

	function close(saved: SavedPlaylist | null) {
		if (finished) return;
		finished = true;
		if (dialog?.open) dialog.close();
		if (opener?.isConnected) opener.focus();
		if (saved) onsaved(saved);
		else oncancel();
	}

	async function save(replace: boolean) {
		touched = true;
		if (problem || busy) return;
		busy = true;
		error = '';
		try {
			const r = await me.saveQueue(player, name, replace);
			if ('saved' in r) close(r.saved);
			else if ('exists' in r) exists = r.exists;
			else error = 'The queue is empty: there’s nothing to save.';
		} catch (e) {
			error = e instanceof ApiError || e instanceof RangeError ? `Couldn’t save the playlist: ${e.message}` : 'Couldn’t save the playlist. Check the connection and try again.';
		} finally {
			busy = false;
		}
	}

	function submit(e: SubmitEvent) {
		e.preventDefault();
		void save(false);
	}

	function rename() {
		exists = null;
		queueMicrotask(() => input?.focus());
	}
</script>

<dialog
	bind:this={dialog}
	aria-modal="true"
	aria-labelledby={`${id}-t`}
	onclose={() => close(null)}
	oncancel={(e) => {
		e.preventDefault();
		close(null);
	}}
	onclick={(e) => e.target === dialog && close(null)}
	data-save-queue-dialog
>
	<div class="box">
		<h2 id={`${id}-t`}>Save queue as playlist</h2>
		{#if exists}
			<p role="alert" data-save-exists>A playlist called “{exists.name}” already exists. Replace it with this queue?</p>
			{#if error}<p class="error" role="alert">{error}</p>{/if}
			<div class="actions">
				<button type="button" class="btn" onclick={rename} disabled={busy} data-save-rename>Change name</button>
				<button type="button" class="btn primary danger" onclick={() => save(true)} disabled={busy} data-save-replace>Replace</button>
			</div>
		{:else}
			<form onsubmit={submit} novalidate>
				<label class="field">
					<span>Name</span>
					<input
						bind:this={input}
						bind:value={name}
						oninput={() => (touched = true)}
						maxlength={PLAYLIST_NAME_MAX * 2}
						autocomplete="off"
						aria-invalid={touched && !!problem}
						aria-describedby={touched && problem ? `${id}-p` : undefined}
						data-save-name
					/>
				</label>
				{#if touched && problem}<p class="problem" id={`${id}-p`} data-save-problem>{problem}</p>{/if}
				{#if error}<p class="error" role="alert" data-save-error>{error}</p>{/if}
				<div class="actions">
					<button type="button" class="btn" onclick={() => close(null)} data-save-cancel>Cancel</button>
					<button type="submit" class="btn primary" disabled={busy || (touched && !!problem)} data-save-ok>{busy ? 'Saving…' : 'Save'}</button>
				</div>
			</form>
		{/if}
	</div>
</dialog>

<style>
	dialog {
		border: 0;
		padding: 0;
		background: transparent;
		color: var(--fg);
		max-width: min(92vw, 420px);
		width: 100%;
	}
	dialog::backdrop {
		background: rgb(0 0 0 / 45%);
	}
	.box {
		background: var(--bg-raised);
		color: var(--fg);
		border-radius: var(--radius);
		padding: 1.1rem 1.1rem 1rem;
		box-shadow: var(--shadow);
	}
	h2 {
		margin: 0 0 0.75rem;
		font-size: 1.1rem;
	}
	p {
		margin: 0 0 1rem;
	}
	.field {
		display: grid;
		gap: 0.3rem;
		margin-bottom: 0.75rem;
	}
	.field input {
		min-height: 44px;
	}
	.problem {
		color: var(--danger, #b3261e);
		font-size: 0.9rem;
		margin-top: -0.4rem;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	.actions .btn {
		min-height: 44px;
	}
	.danger {
		background: var(--danger, #b3261e);
		border-color: var(--danger, #b3261e);
		color: #fff;
	}
</style>
