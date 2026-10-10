<script lang="ts">
	import { ApiError } from '$lib/api/http';
	import { session } from '$lib/state/app.svelte';

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
			if (err instanceof ApiError && err.totpRequired) {
				needCode = true;
				message = 'Enter the code from your authenticator app.';
				queueMicrotask(() => codeInput?.focus());
			} else if (err instanceof ApiError && err.status === 429) {
				message = 'Too many attempts. Wait a while and try again.';
			} else if (err instanceof ApiError && err.status === 401) {
				message = needCode ? 'That code or password is not right.' : 'Username or password is not right.';
			} else {
				message = 'Could not reach the server. Check your connection.';
			}
		} finally {
			busy = false;
		}
	}
</script>

<main class="login">
	<form class="card" onsubmit={submit} autocomplete="on">
		<div class="brand">
			<span class="logo" aria-hidden="true"></span>
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
				<span>Two-step code</span>
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
		display: grid;
		place-items: center;
		padding: 1rem;
		background:
			radial-gradient(60rem 40rem at 10% -10%, var(--accent-soft), transparent 60%),
			var(--bg);
	}
	form {
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
		width: 34px;
		height: 34px;
		border-radius: 10px;
		background: conic-gradient(from 200deg, var(--accent), #f59e0b, var(--accent));
		box-shadow: inset 0 0 0 9px var(--bg-raised);
	}
</style>
