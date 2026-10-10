<script lang="ts">
	import { app, session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import * as auth from '$lib/api/auth';
	import { ApiError } from '$lib/api/http';
	import Icon from '$lib/components/Icon.svelte';
	import AppearancePicker from '$lib/components/AppearancePicker.svelte';
	import { detect } from '$lib/player/capabilities';
	import { MODE_LABELS, chooseMode, detectEnvironment, loadPreference, savePreference, type ModePreference } from '$lib/player/mode';

	// Built in: every browser is checked, and the best playback mode chosen automatically.
	const caps = detect();
	const env = detectEnvironment();
	let pref = $state<ModePreference>(loadPreference());
	const planned = $derived(chooseMode(caps, env, pref));
	const active = $derived(device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting');
	function setPref(p: ModePreference) {
		pref = p;
		savePreference(p);
	}

	let account = $state<auth.Account | null>(null);
	let setup = $state<auth.TotpSetup | null>(null);
	let code = $state('');
	let offPassword = $state('');
	let offCode = $state('');
	let msg = $state('');

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
	const mb = (b?: number) => (b ? `${(b / 1048576).toFixed(1)} MB` : '–');

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
		<div class="mode">
			<p>
				<strong>Playback: {MODE_LABELS[(active && device.choice ? device.choice : planned).mode]}</strong>
				<span class="muted"> — {(active && device.choice ? device.choice : planned).reason}</span>
			</p>
			<div class="row" role="radiogroup" aria-label="Playback mode">
				{#each [['auto', 'Automatic'], ['worklet', 'Web Audio'], ['media', 'Media element']] as const as [value, label] (value)}
					<label class="pill choice" class:on={pref === value}>
						<input type="radio" name="playmode" {value} checked={pref === value} onchange={() => setPref(value)} class="sr-only" />{label}
					</label>
				{/each}
			</div>
			{#if active && device.choice && planned.mode !== device.choice.mode}
				<p class="muted small">Takes effect the next time you start playing here.
					<button class="btn" onclick={async () => { device.stop(); await device.start(); }}>Restart player now</button></p>
			{/if}
		</div>
		{#if device.state !== 'off'}
			<dl class="diag" aria-label="Player diagnostics">
				<dt>Output</dt><dd data-diag="mode">{device.detail.mode === 'media' ? 'Media element' : device.detail.mode === 'worklet' ? 'Web Audio' : '–'}</dd>
				<dt>State</dt><dd data-diag="output">{device.detail.output ?? '–'}</dd>
				<dt>Buffered audio</dt><dd data-diag="buffered">{((device.detail.buffered_ms ?? 0) / 1000).toFixed(1)} s</dd>
				<dt>Downloaded, not yet decoded</dt><dd data-diag="input">{mb(device.detail.input_bytes)}</dd>
				<dt>Player memory</dt><dd data-diag="memory">{mb(device.detail.wasm_bytes)}</dd>
				<dt>Sample rate</dt><dd>{device.detail.sample_rate ? `${device.detail.sample_rate / 1000} kHz` : '–'}</dd>
			</dl>
		{/if}
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
		<h2>Playback capabilities</h2>
		<p class="muted">What this browser supports; the automatic playback choice is based on this.</p>
		{#if caps}
			<dl class="diag">
				<dt>Managed Media Source</dt><dd>{caps.managedMediaSource ? 'yes' : 'no'}</dd>
				<dt>Media Source</dt><dd>{caps.mediaSource ? 'yes' : 'no'}</dd>
				<dt>AudioWorklet</dt><dd>{caps.audioWorklet ? 'yes' : 'no'}</dd>
				<dt>Audio session</dt><dd>{caps.audioSession ? 'yes' : 'no'}</dd>
			</dl>
			<table class="caps">
				<thead><tr><th>Format</th><th>Managed</th><th>Classic</th></tr></thead>
				<tbody>
					{#each caps.types as t (t.type)}
						<tr><td><code>{t.type}</code></td><td>{t.managed ? 'yes' : '–'}</td><td>{t.classic ? 'yes' : '–'}</td></tr>
					{/each}
				</tbody>
			</table>
			<p class="muted small">{caps.userAgent}</p>
		{/if}
	</div>

	<div class="card">
		<h2>Appearance</h2>
		<AppearancePicker />
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
	.diag {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: 0.2rem 1rem;
		margin: 1rem 0 0;
		font-size: 0.9rem;
	}
	.diag dt {
		color: var(--fg-muted);
	}
	.diag dd {
		margin: 0;
		font-variant-numeric: tabular-nums;
	}
	.mode {
		margin: 1rem 0 0;
		display: grid;
		gap: 0.6rem;
	}
	.mode p {
		margin: 0;
	}
	.caps {
		width: 100%;
		margin-top: 0.75rem;
		border-collapse: collapse;
		font-size: 0.85rem;
	}
	.caps th,
	.caps td {
		text-align: left;
		padding: 0.25rem 0.5rem;
		border-bottom: 1px solid var(--line);
	}
	.caps code {
		font-family: var(--mono);
		font-size: 0.8rem;
		word-break: break-all;
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
