<script lang="ts">
	// "Add another device": a one-time code that signs another device (the Android app) in
	// as this user, shown as a QR code holding the pairing link and as text to type. The
	// gateway asks for the password and makes a few an hour; each lasts 10 minutes, once.
	import { ApiError } from '$lib/api/http';
	import type { PairingCode } from '$lib/api/auth';
	import { encodeQr, groupPairingCode, pairingLink, qrPath } from '$lib/qr/qr.js';
	import Icon from './Icon.svelte';

	let {
		create,
		server,
		now = () => Date.now(),
	}: {
		/** Ask the gateway for a code (with the account password). */
		create: (password: string) => Promise<PairingCode>;
		/** This app's own address (`location.origin`): the server the link names. */
		server: string;
		now?: () => number;
	} = $props();

	let password = $state('');
	let busy = $state(false);
	let error = $state('');
	let shown = $state<{ code: string; link: string | null; ends: number } | null>(null);
	let left = $state(0);

	const qr = $derived.by(() => {
		if (!shown?.link) return null;
		const symbol = encodeQr(shown.link);
		return { n: symbol.size + 8, d: qrPath(symbol, 4) };
	});
	const expired = $derived(shown !== null && left <= 0);

	$effect(() => {
		if (!shown) return;
		const ends = shown.ends;
		const update = () => (left = Math.max(0, Math.round((ends - now()) / 1000)));
		update();
		const timer = setInterval(update, 1000);
		return () => clearInterval(timer);
	});

	const countdown = (s: number) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (busy) return;
		busy = true;
		error = '';
		try {
			const r = await create(password);
			const code = groupPairingCode(r.code);
			if (!code) throw new Error('The server sent no usable code.');
			const secs = Number.isFinite(r.expires_in) && r.expires_in > 0 && r.expires_in <= 3600 ? r.expires_in : 600;
			shown = { code, link: pairingLink(server, r.code), ends: now() + secs * 1000 };
		} catch (err) {
			error =
				err instanceof ApiError && err.status === 401
					? 'Password not right.'
					: err instanceof ApiError && err.status === 429
						? 'Too many codes or new devices recently. Try again later.'
						: (err as Error).message;
		} finally {
			password = '';
			busy = false;
		}
	}

	function done() {
		shown = null;
		error = '';
	}
</script>

<h3>Add another device</h3>
{#if shown}
	<div class="pairing" data-pairing>
		{#if !expired}
			<p>In the Lyrion Anywhere app on the other device, choose <strong>Scan QR code</strong> and point the camera here, or type the code.</p>
			{#if qr}
				<svg class="qr" viewBox="0 0 {qr.n} {qr.n}" width="232" height="232" role="img" aria-label="QR code to sign in the Lyrion Anywhere app" shape-rendering="crispEdges" data-qr>
					<rect width={qr.n} height={qr.n} fill="#fff" />
					<path d={qr.d} fill="#000" />
				</svg>
			{:else}
				<p class="muted" data-no-qr>No QR code: this page isn't open on an https:// address the app can use. Enter the server address in the app, then type the code.</p>
			{/if}
			<p class="code" data-code>{shown.code}</p>
			<p class="muted small" data-countdown>Expires in {countdown(left)}. It works once, and signs in as you.</p>
		{:else}
			<p class="code expired" data-code>{shown.code}</p>
			<p class="muted small" data-countdown>This code has expired. Make a new one if you still need it.</p>
		{/if}
		<button class="btn" type="button" onclick={done}>Done</button>
	</div>
{:else}
	<p class="muted">Sign in the Android app (or another device) without typing your password there: show a one-time code here and scan it.</p>
	<form onsubmit={submit} class="inline" data-add-device>
		<label class="field"><span>Password</span><input type="password" bind:value={password} autocomplete="current-password" required /></label>
		<button class="btn" type="submit" disabled={busy || !password}><Icon name="plus" size={18} />Show a code</button>
	</form>
{/if}
{#if error}<p class="note" role="status" data-add-device-error>{error}</p>{/if}

<style>
	.pairing {
		display: grid;
		gap: 0.6rem;
		justify-items: start;
	}
	.pairing p {
		margin: 0;
	}
	.qr {
		display: block;
		border-radius: var(--radius-sm);
		max-width: 100%;
		height: auto;
	}
	.code {
		font-family: var(--mono);
		font-size: 1.8rem;
		font-weight: 700;
		letter-spacing: 0.08em;
	}
	.expired {
		text-decoration: line-through;
		opacity: 0.5;
	}
	.inline {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
		align-items: end;
		margin-top: 0.75rem;
	}
	.small {
		font-size: 0.85rem;
	}
	.note {
		padding: 0.6rem 0.9rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
	}
</style>
