<script lang="ts">
	// "Your devices" (Settings › Account): the devices signed in to this account, this one
	// marked, each other one with Sign out (after a confirm), and Sign out all other devices.
	// Signing out takes effect at once: that device has to sign in again. One message at a
	// time, announced.
	import type { MyDevice } from '$lib/api/auth';
	import { cleanDevices, deviceLine, devicesError } from '$lib/account/devices';
	import { ApiError } from '$lib/api/http';
	import ConfirmDialog from './ConfirmDialog.svelte';

	let {
		load,
		signOut,
		signOutOthers,
		now = () => Date.now(),
	}: {
		/** The raw list from the gateway (through the session, which adds the token). */
		load: () => Promise<unknown>;
		/** Sign out one other device. */
		signOut: (id: number) => Promise<void>;
		/** Sign out every other device; how many went. */
		signOutOthers: () => Promise<number>;
		now?: () => number;
	} = $props();

	let devices = $state<MyDevice[] | null>(null);
	let busy = $state(false);
	/** The one message shown: what happened last, or what went wrong. */
	let message = $state<{ text: string; error: boolean } | null>(null);
	/** Waiting for a yes: one device, or all the others. */
	let confirming = $state<MyDevice | 'others' | null>(null);

	const others = $derived(devices?.filter((d) => !d.current) ?? []);

	/** Each load is numbered: an answer to an older one than the last asked for is dropped. */
	let seq = 0;

	/** Load the list; what went wrong, or null. A stale answer changes nothing. */
	async function refresh(): Promise<unknown> {
		const mine = ++seq;
		try {
			const list = cleanDevices(await load());
			if (mine === seq) devices = list;
			return null;
		} catch (err) {
			return mine === seq ? err : null;
		}
	}

	/** The first load, or Retry after it failed: a failure is the message, with Retry. */
	async function initial() {
		busy = true;
		message = null;
		const err = await refresh();
		if (err) message = { text: devicesError(err), error: true };
		busy = false;
	}
	$effect(() => {
		void initial();
	});

	/** Take devices off the list here, so it is right even if the refresh after fails. */
	function drop(gone: (d: MyDevice) => boolean) {
		if (devices) devices = devices.filter((d) => !gone(d));
	}

	async function confirmed() {
		const what = confirming;
		confirming = null;
		if (!what || busy) return;
		busy = true;
		message = null;
		try {
			if (what === 'others') {
				const n = await signOutOthers();
				drop((d) => !d.current);
				message = { text: n === 1 ? '1 other device was signed out.' : `${n} other devices were signed out.`, error: false };
			} else {
				await signOut(what.id);
				drop((d) => d.id === what.id);
				message = { text: `“${what.name}” was signed out.`, error: false };
			}
		} catch (err) {
			// Gone already: as good as done.
			if (err instanceof ApiError && err.status === 404) {
				if (what !== 'others') drop((d) => d.id === what.id);
				message = { text: devicesError(err), error: false };
			} else message = { text: devicesError(err), error: true };
		}
		// Busy until the list is fresh, so no second action races this refresh. A failed
		// refresh leaves the message about what was done (the list is already right).
		try {
			await refresh();
		} finally {
			busy = false;
		}
	}
</script>

<div class="your-devices" data-your-devices>
	<p class="muted small">Devices signed in to your account. Signing one out ends its sign-in at once: it has to sign in again to play.</p>
	{#if devices === null}
		{#if !message}<p class="muted" data-devices-loading>Loading…</p>{/if}
		{#if message?.error && !busy}<button class="btn" onclick={() => void initial()} data-devices-retry>Try again</button>{/if}
	{:else}
		<ul class="list" data-device-list>
			{#each devices as d (d.id)}
				<li class="device" data-device={d.id}>
					<div class="grow">
						<div class="name">
							<strong>{d.name}</strong>
							{#if d.current}<span class="badge" data-this-device>This device</span>{/if}
						</div>
						<div class="muted small">{deviceLine(d, now())}</div>
					</div>
					{#if !d.current}
						<button class="btn" disabled={busy} onclick={() => (confirming = d)} aria-label={`Sign out ${d.name}`} data-sign-out-device={d.id}>Sign out</button>
					{/if}
				</li>
			{/each}
		</ul>
		{#if others.length > 0}
			<button class="btn" disabled={busy} onclick={() => (confirming = 'others')} data-sign-out-others>Sign out all other devices</button>
		{:else}
			<p class="muted small" data-no-other-devices>No other devices are signed in.</p>
		{/if}
	{/if}
	<div aria-live="polite" role="status" data-devices-status>
		{#if message}<p class:error={message.error} data-devices-message>{message.text}</p>{/if}
	</div>
</div>

{#if confirming === 'others'}
	<ConfirmDialog
		title="Sign out all other devices?"
		message={`${others.length === 1 ? 'One device' : `${others.length} devices`} will have to sign in again. This device stays signed in.`}
		confirmLabel="Sign out all"
		danger
		onconfirm={confirmed}
		oncancel={() => (confirming = null)}
	/>
{:else if confirming}
	<ConfirmDialog title={`Sign out “${confirming.name}”?`} message="It will have to sign in again before it can play." confirmLabel="Sign out" danger onconfirm={confirmed} oncancel={() => (confirming = null)} />
{/if}

<style>
	.your-devices {
		display: grid;
		gap: 0.75rem;
		justify-items: start;
	}
	.your-devices p {
		margin: 0;
	}
	.list {
		list-style: none;
		margin: 0;
		padding: 0;
		width: 100%;
		display: grid;
		gap: 2px;
	}
	.device {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 52px;
		padding: 0.4rem 0;
	}
	.grow {
		flex: 1;
		min-width: 0;
	}
	.name {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 0.5rem;
		overflow-wrap: anywhere;
	}
	.badge {
		font-size: 0.75rem;
		font-weight: 600;
		padding: 0.1rem 0.45rem;
		border-radius: 999px;
		background: var(--bg-sunken);
		color: var(--fg-muted);
	}
	.btn {
		min-height: 44px;
		flex: none;
	}
	.small {
		font-size: 0.85rem;
	}
	.error {
		color: var(--danger);
		font-weight: 550;
	}
</style>
