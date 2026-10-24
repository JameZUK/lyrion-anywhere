<script lang="ts">
	import { ApiError } from '#lib/api/http';
	import { loginError } from '#lib/account/login';
	import { session } from '#lib/state/app.svelte';
	import InsecureNotice from './InsecureNotice.svelte';

	let username = $state('');
	let password = $state('');
	let code = $state('');
	let needCode = $state(false);
	let busy = $state(false);
	let message = $state('');
	let codeInput = $state<HTMLInputElement>();

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		message = '';
		try {
			await session.signIn(username.trim(), password, needCode ? code.trim() : undefined);
			password = '';
			code = '';
		} catch (err) {
			message = loginError(err, needCode);
			if (err instanceof ApiError && err.totpRequired) {
				needCode = true;
				queueMicrotask(() => codeInput?.focus());
			}
		} finally {
			busy = false;
		}
	}
</script>

<main class="login">
	<!-- Before the password is typed: say when the connection isn't encrypted. -->
	<InsecureNotice />
	<form class="card" onsubmit={submit} autocomplete="on">
		<div class="brand">
			<img class="logo" src="./icons/icon.svg" alt="" width="40" height="40" />
			<h1>Lyrion Anywhere</h1>
		</div>
		<p class="muted">Sign in to play your music from anywhere.</p>
		<label class="field">
			<span>Username</span>
			<input bind:value={username} autocomplete="username" autocapitalize="none" spellcheck="false" required />
		</label>
		<label class="field">
			<span>Password</span>
			<input type="password" bind:value={password} autocomplete="current-password" required />
		</label>
		{#if needCode}
			<label class="field">
				<span>Authenticator code</span>
				<input
					bind:this={codeInput}
					bind:value={code}
					inputmode="numeric"
					autocomplete="one-time-code"
					pattern="[0-9]{6}"
					maxlength="6"
					required
				/>
			</label>
		{/if}
		{#if message}<p class="msg" role="alert">{message}</p>{/if}
		<button class="btn primary" type="submit" disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
	</form>
</main>

<style>
	.login {
		min-height: 100vh;
		min-height: 100dvh;
		/* A column: the notice (if any) at the top, the form centred in the rest. */
		display: flex;
		flex-direction: column;
		align-items: center;
		padding: 1rem;
		background:
			radial-gradient(60rem 40rem at 10% -10%, var(--accent-soft), transparent 60%),
			var(--bg);
	}
	form {
		margin: auto;
		width: min(100%, 380px);
		display: grid;
		gap: 1rem;
		box-shadow: var(--shadow);
		padding: 2rem;
	}
	.brand {
		display: flex;
		align-items: center;
		gap: 0.7rem;
	}
	h1 {
		margin: 0;
		font-size: 1.4rem;
	}
	p {
		margin: 0;
	}
	.msg {
		color: var(--danger);
	}
	.btn {
		justify-content: center;
		padding: 0.75em;
	}
	.logo {
		width: 40px;
		height: 40px;
		flex: none;
	}
</style>
