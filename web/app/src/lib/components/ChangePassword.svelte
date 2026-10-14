<script lang="ts">
	// "Change password" (Settings › Account). Asks for the current password (and a two-step
	// code when that is on), checks the new one before sending, and afterwards lists the
	// other devices that stay signed in: each holds its own sign-in, not the password.
	import type { OwnDevice, PasswordChange, PasswordChanged } from '$lib/api/auth';
	import { MIN_PASSWORD, changeError, deviceDetail, formProblem } from '$lib/account/change-password';

	let {
		username,
		totp,
		change,
		now = () => Date.now(),
		heading = true,
	}: {
		/** The signed-in user (for password managers: the hidden username field). */
		username: string;
		/** Two-step verification is on: ask for a code. */
		totp: boolean;
		/** Send the change (through the session, which adds the token). */
		change: (c: PasswordChange) => Promise<PasswordChanged>;
		now?: () => number;
		/** Its own heading (not inside a sheet that names it). */
		heading?: boolean;
	} = $props();

	let current = $state('');
	let next = $state('');
	let confirm = $state('');
	let code = $state('');
	let signOutOthers = $state(false);
	let busy = $state(false);
	let error = $state('');
	/** The server asked for a code although the account said two-step was off. */
	let askCode = $state(false);
	let done = $state<{ signedOut: number; others: OwnDevice[] } | null>(null);

	const needCode = $derived(totp || askCode);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (busy) return;
		error = '';
		done = null;
		const problem = formProblem({ current, next, confirm, totp: needCode, code });
		if (problem) {
			error = problem;
			return;
		}
		busy = true;
		try {
			const r = await change({ current, new: next, code: needCode ? code.replace(/\s/g, '') : undefined, signOutOthers });
			done = { signedOut: Number(r?.signed_out) || 0, others: Array.isArray(r?.other_devices) ? r.other_devices : [] };
			current = next = confirm = '';
			signOutOthers = false;
		} catch (err) {
			const r = changeError(err);
			error = r.message;
			if (r.needCode) askCode = true;
		} finally {
			code = '';
			busy = false;
		}
	}
</script>

{#if heading}<h3>Change password</h3>{/if}
<form onsubmit={submit} class="change-password" novalidate data-change-password>
	<!-- For password managers: which account this new password belongs to. -->
	<input type="text" name="username" autocomplete="username" value={username} readonly hidden aria-hidden="true" tabindex="-1" />
	<label class="field"><span>Current password</span><input type="password" name="current-password" bind:value={current} autocomplete="current-password" required data-current /></label>
	<label class="field"
		><span>New password</span><input type="password" name="new-password" bind:value={next} autocomplete="new-password" minlength={MIN_PASSWORD} required aria-describedby="new-password-rule" data-new /></label
	>
	<label class="field"><span>New password again</span><input type="password" name="confirm-password" bind:value={confirm} autocomplete="new-password" required data-confirm /></label>
	{#if needCode}
		<label class="field"><span>Authenticator code</span><input bind:value={code} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="7" required data-code /></label>
	{/if}
	<p class="muted small" id="new-password-rule">At least {MIN_PASSWORD} characters.</p>
	<label class="check">
		<input type="checkbox" bind:checked={signOutOthers} data-sign-out-others />
		<span>Also sign out my other devices</span>
	</label>
	<button class="btn" type="submit" disabled={busy} data-change-submit>{busy ? 'Changing…' : 'Change password'}</button>
</form>
<div aria-live="polite" role="status" data-change-status>
	{#if error}
		<p class="error" data-change-error>{error}</p>
	{:else if done}
		<p data-change-done>Your password has been changed. This device stays signed in.</p>
		{#if done.signedOut > 0}
			<p class="muted small" data-signed-out>{done.signedOut} other device{done.signedOut === 1 ? ' was' : 's were'} signed out.</p>
		{/if}
		{#if done.others.length > 0}
			<p class="muted small">These devices stay signed in: they don’t need the new password. To sign one out, use Sign out on it, or change your password again with “Also sign out my other devices” ticked.</p>
			<ul class="devices" data-other-devices>
				{#each done.others as d (d.id)}
					<li><strong>{d.name}</strong>{#if deviceDetail(d, now())}<span class="muted small"> — {deviceDetail(d, now())}</span>{/if}</li>
				{/each}
			</ul>
		{:else if done.signedOut === 0}
			<p class="muted small" data-no-others>No other devices are signed in.</p>
		{/if}
	{/if}
</div>

<style>
	.change-password {
		display: grid;
		gap: 0.6rem;
		justify-items: start;
		margin-top: 0.75rem;
	}
	.change-password p {
		margin: 0;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		min-height: 44px;
	}
	.error {
		color: var(--danger);
		font-weight: 550;
	}
	.devices {
		margin: 0.25rem 0 0;
		padding-left: 1.2rem;
	}
</style>
