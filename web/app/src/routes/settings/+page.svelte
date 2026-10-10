<script lang="ts">
	import { app, session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import * as auth from '$lib/api/auth';
	import { ApiError } from '$lib/api/http';
	import Icon from '$lib/components/Icon.svelte';

	let account = $state<auth.Account | null>(null);
	let setup = $state<auth.TotpSetup | null>(null);
	let code = $state('');
	let offPassword = $state('');
	let offCode = $state('');
	let msg = $state('');
	let theme = $state<'system' | 'light' | 'dark'>('system');

	try {
		theme = (localStorage.getItem('anywhere.theme') as typeof theme) ?? 'system';
	} catch {
		/* storage blocked */
	}
	$effect(() => {
		const root = document.documentElement;
		if (theme === 'system') root.removeAttribute('data-theme');
		else root.dataset.theme = theme;
		try {
			localStorage.setItem('anywhere.theme', theme);
		} catch {
			/* not essential */
		}
	});

	async function loadAccount() {
		try {
			account = await session.authed((t) => auth.account(t));
		} catch (e) {
			msg = (e as Error).message;
		}
	}
	$effect(() => {
		void loadAccount();
	});

	async function beginTotp() {
		msg = '';
		try {
			setup = await session.authed((t) => auth.totpSetup(t));
		} catch (e) {
			msg = (e as Error).message;
		}
	}
	async function confirmTotp(e: SubmitEvent) {
		e.preventDefault();
		try {
			await session.authed((t) => auth.totpConfirm(t, code.trim()));
			setup = null;
			code = '';
			msg = 'Two-step verification is on.';
			await loadAccount();
		} catch (err) {
			msg = err instanceof ApiError && err.status === 400 ? 'That code is not right. Try the next one.' : (err as Error).message;
		}
	}
	async function disableTotp(e: SubmitEvent) {
		e.preventDefault();
		try {
			await session.authed((t) => auth.totpDisable(t, offPassword, offCode.trim()));
			msg = 'Two-step verification is off.';
			await loadAccount();
		} catch (err) {
			msg = err instanceof ApiError && err.status === 401 ? 'Password or code not right.' : (err as Error).message;
		} finally {
			offPassword = '';
			offCode = '';
		}
	}
	// The QR code is an SVG the gateway generates; shown as an image, never inserted as markup.
	const qr = $derived(setup ? `data:image/svg+xml;base64,${btoa(setup.qr_svg)}` : '');
</script>

<section class="page settings">
	<h1>Settings</h1>
	{#if msg}<p class="note" role="status">{msg}</p>{/if}

	<div class="card">
		<h2>This device</h2>
		<p class="muted">Play music in this browser. It appears in LMS as a player named “{session.current?.device.name}”.</p>
		<div class="row">
			{#if device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting'}
				<span class="pill on">{device.state === 'connected' ? 'Playing here' : device.state === 'starting' ? 'Starting…' : 'Reconnecting…'}</span>
				<button class="btn" onclick={() => device.stop()}>Stop</button>
			{:else}
				<button class="btn primary" onclick={() => device.start()} disabled={!device.supported}><Icon name="device" size={18} />Play on this device</button>
			{/if}
		</div>
		{#if device.log.length}
			<details>
				<summary>Connection log</summary>
				<pre>{device.log.join('\n')}</pre>
			</details>
		{/if}
	</div>

	<div class="card">
		<h2>Account</h2>
		<p>Signed in as <strong>{account?.username ?? session.current?.user.username}</strong> ({account?.role ?? session.current?.user.role}).</p>
		<h3>Two-step verification: {account ? (account.totp_enabled ? 'on' : 'off') : '…'}</h3>
		{#if account && !account.totp_enabled && !setup}
			<p class="muted">Ask for a code from an authenticator app at every sign-in.</p>
			<button class="btn" onclick={beginTotp}>Turn on</button>
		{:else if setup}
			<p>Scan this with your authenticator app, or type the key:</p>
			<img class="qr" src={qr} alt="QR code for your authenticator app" width="200" height="200" />
			<code class="key">{auth.groupKey(setup.secret)}</code>
			<form onsubmit={confirmTotp} class="inline">
				<label class="field"><span>Code it shows</span><input bind:value={code} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required /></label>
				<button class="btn primary" type="submit">Confirm</button>
			</form>
		{:else if account?.totp_enabled}
			<form onsubmit={disableTotp} class="inline">
				<label class="field"><span>Password</span><input type="password" bind:value={offPassword} autocomplete="current-password" required /></label>
				<label class="field"><span>Current code</span><input bind:value={offCode} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required /></label>
				<button class="btn" type="submit">Turn off</button>
			</form>
		{/if}
	</div>

	<div class="card">
		<h2>Appearance</h2>
		<div class="row" role="radiogroup" aria-label="Theme">
			{#each ['system', 'light', 'dark'] as const as t (t)}
				<label class="pill choice" class:on={theme === t}><input type="radio" name="theme" value={t} bind:group={theme} class="sr-only" />{t[0]!.toUpperCase() + t.slice(1)}</label>
			{/each}
		</div>
	</div>

	<div class="card">
		<button class="btn" onclick={() => { device.stop(); void session.signOut(); }}><Icon name="power" size={18} />Sign out</button>
		<p class="muted small">Signing out removes this device and its player.</p>
	</div>
	<p class="muted small">Connection: {app.events}</p>
</section>

<style>
	.settings {
		display: grid;
		gap: 1.25rem;
		max-width: 760px;
	}
	.card h2 {
		margin-top: 0;
	}
	h3 {
		font-size: 1rem;
		margin-top: 1.25rem;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		align-items: center;
	}
	.pill {
		padding: 0.4em 0.9em;
		border-radius: 999px;
		border: 1px solid var(--line);
		font-weight: 550;
	}
	.pill.on {
		background: var(--accent-soft);
		border-color: var(--accent);
	}
	.choice {
		cursor: pointer;
	}
	.choice:focus-within {
		outline: 2px solid var(--accent);
	}
	.inline {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
		align-items: end;
		margin-top: 0.75rem;
	}
	.qr {
		display: block;
		background: #fff;
		padding: 8px;
		border-radius: var(--radius-sm);
	}
	.key {
		display: inline-block;
		margin-top: 0.5rem;
		font-family: var(--mono);
		letter-spacing: 0.05em;
	}
	pre {
		max-height: 240px;
		overflow: auto;
		font: 0.8rem/1.4 var(--mono);
		background: var(--bg-sunken);
		padding: 0.75rem;
		border-radius: var(--radius-sm);
	}
	.note {
		padding: 0.6rem 0.9rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
	}
	.small {
		font-size: 0.85rem;
		margin: 0.5rem 0 0;
	}
</style>
