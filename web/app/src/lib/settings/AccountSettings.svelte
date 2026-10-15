<script lang="ts">
	// Settings › Account (Wave 3.5): who is signed in, then one row per action (Change password,
	// Two-step verification, Your devices, Add another device), each opening its own dialog
	// (those that need the password ask for it once), and Sign out.
	import { session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import * as auth from '$lib/api/auth';
	import { ApiError } from '$lib/api/http';
	import { downloads, offlinePlayer, plays } from '$lib/offline/index.svelte';
	import { forgetOwner } from '$lib/offline/settings';
	import { signOutHere } from '$lib/account/sign-out';
	import AddDevice from '$lib/components/AddDevice.svelte';
	import ChangePassword from '$lib/components/ChangePassword.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Sheet from '$lib/components/Sheet.svelte';
	import SignOut from '$lib/components/SignOut.svelte';
	import YourDevices from '$lib/components/YourDevices.svelte';

	let account = $state<auth.Account | null>(null);
	let features = $state<auth.Features>({});
	let error = $state('');
	/** The dialog open now, if any. */
	let open = $state<'password' | 'two-step' | 'devices' | 'device' | null>(null);

	async function loadAccount() {
		try {
			account = await session.authed((t) => auth.account(t));
		} catch (e) {
			error = (e as Error).message;
		}
	}
	$effect(() => {
		void loadAccount();
	});
	// Older gateways have no password change or device list: those rows show only when the
	// gateway says so.
	$effect(() => {
		session
			.authed((t) => auth.features(t))
			.then((f) => (features = f))
			.catch(() => {});
	});

	// --- two-step verification --------------------------------------------------------------
	let setup = $state<auth.TotpSetup | null>(null);
	let password = $state('');
	let code = $state('');
	let msg = $state('');

	function openTwoStep() {
		setup = null;
		password = code = msg = '';
		open = 'two-step';
	}
	/** A two-step request is running: no second one starts (a second "Turn on" would replace
	 *  the secret whose QR is on screen). */
	let totpBusy = $state(false);
	async function totpStep(run: () => Promise<void>) {
		if (totpBusy) return;
		totpBusy = true;
		try {
			await run();
		} finally {
			totpBusy = false;
		}
	}
	function beginTotp(e: SubmitEvent) {
		e.preventDefault();
		return totpStep(async () => {
			msg = '';
			try {
				setup = await session.authed((t) => auth.totpSetup(t, password));
			} catch (err) {
				msg = err instanceof ApiError && err.status === 401 ? 'Password not right.' : (err as Error).message;
			} finally {
				password = '';
			}
		});
	}
	function confirmTotp(e: SubmitEvent) {
		e.preventDefault();
		return totpStep(async () => {
			try {
				await session.authed((t) => auth.totpConfirm(t, code.replace(/\s/g, '')));
				setup = null;
				code = '';
				msg = 'Two-step verification is on.';
				await loadAccount();
			} catch (err) {
				msg = err instanceof ApiError && err.status === 400 ? 'That code is not right. Try the next one.' : (err as Error).message;
			}
		});
	}
	function disableTotp(e: SubmitEvent) {
		e.preventDefault();
		return totpStep(async () => {
			try {
				await session.authed((t) => auth.totpDisable(t, password, code.replace(/\s/g, '')));
				msg = 'Two-step verification is off.';
				await loadAccount();
			} catch (err) {
				msg = err instanceof ApiError && err.status === 401 ? 'Password or code not right.' : (err as Error).message;
			} finally {
				password = '';
				code = '';
			}
		});
	}
	// The QR code is an SVG the gateway generates; shown as an image, never inserted as markup.
	const qr = $derived(setup ? `data:image/svg+xml;base64,${btoa(setup.qr_svg)}` : '');

	/** Rejects (and stays signed in) if downloads asked to go couldn't be removed. */
	const signOut = (removeDownloads: boolean) => signOutHere({ device, offlinePlayer, downloads, plays, forgetOwner, session }, removeDownloads);
</script>

<p>Signed in as <strong>{account?.username ?? session.current?.user.username}</strong> ({account?.role ?? session.current?.user.role}).</p>
{#if error}<p class="error" role="alert">{error}</p>{/if}

<div class="rows">
	{#if account && features.change_password}
		<button class="row" onclick={() => (open = 'password')} aria-haspopup="dialog" data-account-row="password">
			<Icon name="pencil" /><span class="grow">Change password</span><span class="chev" aria-hidden="true">›</span>
		</button>
	{/if}
	<button class="row" onclick={openTwoStep} aria-haspopup="dialog" disabled={!account} data-account-row="two-step">
		<Icon name="device" /><span class="grow">Two-step verification</span><span class="value muted">{account ? (account.totp_enabled ? 'On' : 'Off') : '…'}</span><span class="chev" aria-hidden="true">›</span>
	</button>
	{#if features.devices}
		<button class="row" onclick={() => (open = 'devices')} aria-haspopup="dialog" data-account-row="devices">
			<Icon name="device" /><span class="grow">Your devices</span><span class="chev" aria-hidden="true">›</span>
		</button>
	{/if}
	<button class="row" onclick={() => (open = 'device')} aria-haspopup="dialog" data-account-row="device">
		<Icon name="plus" /><span class="grow">Add another device</span><span class="chev" aria-hidden="true">›</span>
	</button>
</div>

<div class="sign-out">
	<SignOut count={downloads.saved.length} bytes={downloads.usedBytes} onsignout={signOut} />
	<p class="muted small">Signing out removes this device and its player.</p>
</div>

{#if open === 'password' && account}
	<Sheet title="Change password" onclose={() => (open = null)}>
		<ChangePassword username={account.username} totp={account.totp_enabled} change={(c) => session.authed((t) => auth.changePassword(t, c))} heading={false} />
	</Sheet>
{:else if open === 'two-step' && account}
	<Sheet title="Two-step verification" onclose={() => (open = null)}>
		<div class="two-step" data-two-step>
			{#if !account.totp_enabled && !setup}
				<p class="muted">Ask for a code from an authenticator app at every sign-in.</p>
				<form onsubmit={beginTotp} class="form">
					<label class="field"><span>Password</span><input type="password" bind:value={password} autocomplete="current-password" required /></label>
					<button class="btn primary" type="submit" disabled={totpBusy}>Turn on</button>
				</form>
			{:else if setup}
				<p>Scan this with your authenticator app, or type the key:</p>
				<img class="qr" src={qr} alt="QR code for your authenticator app" width="200" height="200" />
				<code class="key">{auth.groupKey(setup.secret)}</code>
				<form onsubmit={confirmTotp} class="form">
					<label class="field"><span>Authenticator code</span><input bind:value={code} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]{'{'}6,7{'}'}" maxlength="7" required /></label>
					<button class="btn primary" type="submit" disabled={totpBusy}>Confirm</button>
				</form>
			{:else}
				<p class="muted">Two-step verification is on. To turn it off, enter your password and a code.</p>
				<form onsubmit={disableTotp} class="form">
					<label class="field"><span>Password</span><input type="password" bind:value={password} autocomplete="current-password" required /></label>
					<label class="field"><span>Authenticator code</span><input bind:value={code} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]{'{'}6,7{'}'}" maxlength="7" required /></label>
					<button class="btn" type="submit" disabled={totpBusy}>Turn off</button>
				</form>
			{/if}
			<p class="note" role="status" data-two-step-status>{msg}</p>
		</div>
	</Sheet>
{:else if open === 'devices'}
	<Sheet title="Your devices" onclose={() => (open = null)}>
		<YourDevices
			load={() => session.authed((t) => auth.myDevices(t))}
			signOut={(id) => session.authed((t) => auth.signOutDevice(t, id))}
			signOutOthers={() => session.authed((t) => auth.signOutOtherDevices(t))}
		/>
	</Sheet>
{:else if open === 'device'}
	<Sheet title="Add another device" onclose={() => (open = null)}>
		<AddDevice create={(pw) => session.authed((t) => auth.createPairingCode(t, pw))} server={globalThis.location?.origin ?? ''} heading={false} />
	</Sheet>
{/if}

<style>
	.rows {
		display: grid;
		gap: 2px;
		margin: 0.75rem 0;
	}
	.row {
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
	.row:hover:not(:disabled) {
		background: var(--bg-sunken);
	}
	.row :global(svg) {
		color: var(--fg-muted);
		flex: none;
	}
	.grow {
		flex: 1;
		font-weight: 550;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.2rem;
	}
	.sign-out {
		margin-top: 1.25rem;
	}
	.form {
		display: grid;
		gap: 0.6rem;
		justify-items: start;
	}
	.qr {
		background: #fff;
		padding: 8px;
		border-radius: var(--radius-sm);
	}
	.key {
		display: block;
		margin: 0.5rem 0;
		font-size: 1.05rem;
		letter-spacing: 0.05em;
	}
	.note:empty {
		display: none;
	}
	.small {
		font-size: 0.85rem;
	}
</style>
