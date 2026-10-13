<script lang="ts">
	import RatingsSettings from '$lib/components/RatingsSettings.svelte';
	import { PREFERENCES, type QualityPreference } from '$lib/player/stream-quality';
	const QUALITY_LABELS: Record<string, string> = { auto: 'Automatic', original: 'Original', high: 'High', low: 'Data saver' };
	import { app, session } from '$lib/state/app.svelte';
	import { device } from '$lib/player/device.svelte';
	import * as auth from '$lib/api/auth';
	import { ApiError } from '$lib/api/http';
	import Icon from '$lib/components/Icon.svelte';
	import AppearancePicker from '$lib/components/AppearancePicker.svelte';
	import { install } from '$lib/pwa/install.svelte';
	import { detect } from '$lib/player/capabilities';
	import { MODE_LABELS, chooseMode, detectEnvironment, loadPreference, savePreference, type ModePreference } from '$lib/player/mode';
	import SignOut from '$lib/components/SignOut.svelte';
	import AddDevice from '$lib/components/AddDevice.svelte';
	import HistorySettings from '$lib/components/HistorySettings.svelte';
	import { version as buildVersion } from '$app/environment';
	import pkg from '../../../package.json';
	import { VOLUME_LABELS, canFollow, planVolume, type VolumeSetting } from '$lib/player/volume';
	import { downloads, offlinePlayer, plays } from '$lib/offline/index.svelte';
	import { forgetOwner, formatBytes, loadPrefs, networkType, requestPersist, savePrefs, storageInfo, type DownloadPrefs, type StorageInfo } from '$lib/offline/settings';
	import { FORMATS, FORMAT_LABELS } from '$lib/offline/types';
	import { loadDefault, saveDefault, type DefaultSetting } from '$lib/player/default-player';
	import { isThisDevice } from '$lib/player/outputs';
	import { textOf } from '$lib/lms/format';
	import { FADE_CHOICES } from '$lib/lms/sleep';
	import { lyrics } from '$lib/state/lyrics';
	import { nowPlaying } from '$lib/nowplaying/store.svelte';
	import { STYLES, STYLE_LABELS, type VisualStyle } from '$lib/visual/draw';

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

	// Which player the app uses when it opens on this device (SPEC D20), per user.
	const user = $derived(session.current?.user.username ?? '');
	let startWith = $state<DefaultSetting>({ kind: 'device' });
	$effect(() => {
		startWith = loadDefault(user);
	});
	const startKey = (d: DefaultSetting) => (d.kind === 'player' ? d.id : d.kind);
	const startPlayers = $derived(app.players.filter((p) => !isThisDevice(p.playerid)));
	function setStartWith(v: string) {
		const d: DefaultSetting = v === 'device' || v === 'last' ? { kind: v } : { kind: 'player', id: v };
		startWith = d;
		saveDefault(user, d);
	}

	// Volume on this device: follows LMS where the browser allows it, otherwise the device's
	// own buttons (see lib/player/volume.ts).
	const volAbility = $derived({ mode: active && device.choice ? device.choice.mode : planned.mode, mediaVolume: device.volumeAbility().mediaVolume });
	const volFollow = $derived(canFollow(volAbility));
	const volPlan = $derived(active && device.volumePlan ? device.volumePlan : planVolume(volAbility, device.volumeSetting));
	const VOLUME_CHOICES: VolumeSetting[] = ['auto', 'follow', 'device'];

	// This device's name: the player name in LMS. Renaming renames that player too.
	const deviceName = $derived(app.session?.device.name ?? '');
	let newName = $state('');
	$effect(() => {
		newName = deviceName;
	});
	async function renameThis(e: SubmitEvent) {
		e.preventDefault();
		const name = auth.validDeviceName(newName);
		if (!name) {
			msg = 'Device names are 1 to 64 characters.';
			return;
		}
		try {
			const r = await session.authed((t) => auth.renameDevice(t, name));
			session.setDeviceName(r.name);
			msg = `This device is now called “${r.name}”.`;
			void app.loadPlayers();
		} catch (err) {
			msg = (err as Error).message;
		}
	}

	let account = $state<auth.Account | null>(null);
	let setup = $state<auth.TotpSetup | null>(null);
	let code = $state('');
	let onPassword = $state('');
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

	async function beginTotp(e: SubmitEvent) {
		e.preventDefault();
		msg = '';
		try {
			setup = await session.authed((t) => auth.totpSetup(t, onPassword));
		} catch (err) {
			msg = err instanceof ApiError && err.status === 401 ? 'Password not right.' : (err as Error).message;
		} finally {
			onPassword = '';
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
	// Downloads (SPEC D5): a default format for each network type, and storage use.
	let prefs = $state<DownloadPrefs>(loadPrefs());
	function setPrefs(p: Partial<DownloadPrefs>) {
		prefs = { ...prefs, ...p };
		savePrefs(prefs);
	}
	const network = networkType();
	let storage = $state<StorageInfo>({});
	$effect(() => {
		void downloads.saved.length;
		let live = true;
		storageInfo().then((i) => live && (storage = i));
		return () => (live = false);
	});
	let confirmRemove = $state(false);
	async function removeAllDownloads() {
		confirmRemove = false;
		offlinePlayer.stop();
		await downloads.removeAll();
		msg = 'Downloads removed from this device.';
	}
	async function keepDownloads() {
		await requestPersist();
		storage = await storageInfo();
	}

	async function signOut(removeDownloads: boolean) {
		device.stop();
		offlinePlayer.stop();
		if (removeDownloads) await downloads.removeAll().catch(() => {});
		// Offline mode no longer offers this account's downloads without signing in.
		forgetOwner();
		await downloads.setUser(null);
		await plays.setUser(null);
		await session.signOut();
	}

	const mb = (b?: number) => (b ? `${(b / 1048576).toFixed(1)} MB` : '–');

	// The QR code is an SVG the gateway generates; shown as an image, never inserted as markup.
	const qr = $derived(setup ? `data:image/svg+xml;base64,${btoa(setup.qr_svg)}` : '');
</script>

<section class="page settings">
	<h1>Settings</h1>
	{#if msg}<p class="note" role="status">{msg}</p>{/if}

	{#if install.way === 'prompt' || install.way === 'ios'}
		<div class="card">
			<h2>Install the app</h2>
			<p>Add Lyrion Anywhere to your home screen: it opens full screen like any other app, and remembers your settings.</p>
			{#if install.way === 'prompt'}
				<button class="btn primary" onclick={() => void install.install()}><Icon name="plus" size={18} />Install</button>
			{:else}
				<ol class="steps">
					<li>Tap the <strong>Share</strong> button (the square with an arrow).</li>
					<li>Choose <strong>Add to Home Screen</strong>.</li>
					<li>Tap <strong>Add</strong>.</li>
				</ol>
			{/if}
		</div>
	{/if}

	<div class="card">
		<h2>Appearance</h2>
		<AppearancePicker />
	</div>

	<div class="card">
		<h2>Ratings</h2>
		<RatingsSettings />
	</div>

	<div class="card">
		<h2>Lyrics</h2>
		<label class="check">
			<input type="checkbox" checked={lyrics.peek} onchange={(e) => lyrics.setPeek(e.currentTarget.checked)} data-lyrics-peek-setting />
			<span>Show the current line on Now playing</span>
		</label>
		<p class="muted small">When a track has synced lyrics. All lyrics are always a tap away with the Lyrics button. Kept in this browser.</p>
	</div>

	<div class="card">
		<h2>Visualiser</h2>
		<label class="check">
			<input type="checkbox" checked={nowPlaying.visualiser} onchange={(e) => nowPlaying.setVisualiser(e.currentTarget.checked)} data-visualiser-setting />
			<span>Show the visualiser on Now playing</span>
		</label>
		<label class="field">
			<span>Style</span>
			<select value={nowPlaying.style} onchange={(e) => nowPlaying.setStyle(e.currentTarget.value as VisualStyle)} data-visualiser-style-setting>
				{#each STYLES as st (st)}<option value={st}>{STYLE_LABELS[st]}</option>{/each}
			</select>
		</label>
		<p class="muted small">In place of the cover, while this browser is the player and Now playing is on screen. Off while your device asks for reduced motion. Kept in this browser.</p>
	</div>

	<div class="card">
		<h2>Sleep timer</h2>
		<label class="field">
			<span>Fade out before it stops</span>
			<select value={app.sleepFade} onchange={(e) => app.setSleepFade(Number(e.currentTarget.value))} data-sleep-fade>
				{#each FADE_CHOICES as f (f)}<option value={f}>{f === 0 ? 'Off' : f < 60 ? `${f} seconds` : f === 60 ? '1 minute' : `${f / 60} minutes`}</option>{/each}
			</select>
		</label>
		<p class="muted small">For this browser's own player; kept for your account in this browser. Other players follow LMS: they fade over the last minute, then stop and turn off.</p>
	</div>

	<div class="card">
		<h2>Streaming quality</h2>
		<label class="field">
			<span>In this browser</span>
			<select value={device.qualityPreference} onchange={(e) => device.setQualityPreference(e.currentTarget.value as QualityPreference)} data-stream-quality>
				{#each PREFERENCES as p (p)}<option value={p}>{QUALITY_LABELS[p]}</option>{/each}
			</select>
		</label>
		<p class="muted small" data-stream-quality-now>
			{#if device.qualityPreference === 'auto'}Now: {QUALITY_LABELS[device.quality]}. Automatic uses High on a mobile connection and Data saver when the browser is asked to save data, where the browser says (Safari doesn't: choose a level there).{/if}
			High (Opus 160 kbps) and Data saver (Opus 96 kbps) re-encode lossless tracks on your server, when it offers this; other tracks are sent as they are. Applies from the next track.
		</p>
	</div>

	<div class="card">
		<h2>Downloads</h2>
		<p class="muted">Download albums to play them without a connection. Downloads stay on this device until you remove them, and are kept apart for each account.</p>
		<div class="formats">
			<label class="field">
				<span>On Wi-Fi{network === 'wifi' ? ' (now)' : ''}</span>
				<select value={prefs.wifi} onchange={(e) => setPrefs({ wifi: e.currentTarget.value as DownloadPrefs['wifi'] })}>
					{#each FORMATS as f (f)}<option value={f}>{FORMAT_LABELS[f]}</option>{/each}
				</select>
			</label>
			<label class="field">
				<span>On mobile data{network === 'cellular' ? ' (now)' : ''}</span>
				<select value={prefs.cellular} onchange={(e) => setPrefs({ cellular: e.currentTarget.value as DownloadPrefs['cellular'] })}>
					{#each FORMATS as f (f)}<option value={f}>{FORMAT_LABELS[f]}</option>{/each}
				</select>
			</label>
		</div>
		<p class="muted small">Original is the file as it is in your library (largest). Opus is about 1 MB a minute. MP3 plays everywhere. If the server can’t convert, Original is used.</p>
		<p data-testid="download-usage">
			<strong>{downloads.saved.length} downloaded {downloads.saved.length === 1 ? 'track' : 'tracks'}, {formatBytes(downloads.usedBytes)}</strong>
			{#if storage.quota}<span class="muted"> · this app uses {formatBytes(storage.usage ?? 0)} of {formatBytes(storage.quota)}</span>{/if}
		</p>
		<div class="row">
			<a class="btn" href="#/downloads"><Icon name="download" size={18} />Open downloads</a>
			{#if storage.persisted === false}
				<button class="btn" onclick={keepDownloads}>Ask the browser to keep downloads</button>
			{/if}
			{#if downloads.saved.length || downloads.items.length}
				{#if confirmRemove}
					<button class="btn" onclick={removeAllDownloads}><Icon name="trash" size={18} />Yes, remove all downloads</button>
					<button class="btn" onclick={() => (confirmRemove = false)}>Cancel</button>
				{:else}
					<button class="btn" onclick={() => (confirmRemove = true)}><Icon name="trash" size={18} />Remove all downloads…</button>
				{/if}
			{/if}
		</div>
	</div>

	<div class="card">
		<h2>This device</h2>
		<p class="muted">Music plays in this browser unless you choose another player. It appears in LMS as a player named “{deviceName}”.</p>
		<form onsubmit={renameThis} class="inline" data-rename>
			<label class="field"><span>Name</span><input bind:value={newName} maxlength="64" autocomplete="off" spellcheck="false" required data-device-name /></label>
			<button class="btn" type="submit" disabled={!auth.validDeviceName(newName) || newName.trim() === deviceName}>Rename</button>
		</form>
		<label class="field start-with">
			<span>When the app opens, play on</span>
			<select value={startKey(startWith)} onchange={(e) => setStartWith(e.currentTarget.value)} data-start-with>
				<option value="device">This device</option>
				<option value="last">The player used last</option>
				{#each startPlayers as p (p.playerid)}
					<option value={p.playerid}>{textOf(p.name)}</option>
				{/each}
				{#if startWith.kind === 'player' && !startPlayers.some((p) => startWith.kind === 'player' && p.playerid === startWith.id)}
					<option value={startKey(startWith)}>A player not connected now</option>
				{/if}
			</select>
		</label>
		{#if !device.supported}
			<p class="muted small">This browser can't play music itself, so the app uses the player used last.</p>
		{/if}
		<div class="row">
			{#if device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting'}
				<span class="pill on">{device.state === 'connected' ? 'Player ready' : device.state === 'starting' ? 'Starting…' : 'Reconnecting…'}</span>
				<button class="btn" onclick={() => device.stop()}>Stop the player</button>
			{:else}
				<button class="btn primary" onclick={() => device.start()} disabled={!device.supported}><Icon name="device" size={18} />Start the player</button>
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
			<p>
				<strong>Volume on this device: {volPlan.follow ? 'follows LMS' : 'device buttons'}</strong>
				<span class="muted"> — {volPlan.reason}</span>
			</p>
			<div class="row" role="radiogroup" aria-label="Volume on this device">
				{#each VOLUME_CHOICES as value (value)}
					{@const off = value === 'follow' && !volFollow.ok}
					<label class="pill choice" class:on={device.volumeSetting === value} class:off aria-disabled={off}>
						<input type="radio" name="volmode" {value} checked={device.volumeSetting === value} disabled={off} onchange={() => device.setVolumeSetting(value)} class="sr-only" />{VOLUME_LABELS[value]}
					</label>
				{/each}
			</div>
			{#if !volFollow.ok}
				<p class="muted small" data-volume-reason>{VOLUME_LABELS.follow} isn't available: {volFollow.why}</p>
			{/if}
			{#if active && device.choice && planned.mode !== device.choice.mode}
				<p class="muted small">Takes effect the next time you start playing here.
					<button class="btn" onclick={async () => { device.stop(); await device.start(); }}>Restart player now</button></p>
			{/if}
		</div>
	</div>

	<div class="card">
		<h2>Listening history</h2>
		<HistorySettings />
	</div>

	<div class="card">
		<h2>Account</h2>
		<p>Signed in as <strong>{account?.username ?? session.current?.user.username}</strong> ({account?.role ?? session.current?.user.role}).</p>
		<h3>Two-step verification: {account ? (account.totp_enabled ? 'on' : 'off') : '…'}</h3>
		{#if account && !account.totp_enabled && !setup}
			<p class="muted">Ask for a code from an authenticator app at every sign-in.</p>
			<form onsubmit={beginTotp} class="inline">
				<label class="field"><span>Password</span><input type="password" bind:value={onPassword} autocomplete="current-password" required /></label>
				<button class="btn" type="submit">Turn on</button>
			</form>
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
		<AddDevice create={(pw) => session.authed((t) => auth.createPairingCode(t, pw))} server={globalThis.location?.origin ?? ''} />
	</div>

	<div class="card">
		<SignOut count={downloads.saved.length} bytes={downloads.usedBytes} onsignout={signOut} />
		<p class="muted small">Signing out removes this device and its player.</p>
	</div>

	<details class="card developer" data-developer>
		<summary>Diagnostics</summary>
		<p class="muted small">For troubleshooting: what this browser and its player report.</p>
		<h3>This device’s player</h3>
		{#if device.state === 'off'}<p class="muted small">The player isn’t running.</p>{/if}
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
		<h3>Playback capabilities</h3>
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
		<p class="muted small">Connection to the server: {app.events}</p>
	</details>

	<div class="card about">
		<h2>About</h2>
		<div class="about-row">
			<img src="./icons/icon.svg" alt="" width="64" height="64" />
			<div>
				<p class="about-name">Lyrion Anywhere</p>
				<p class="muted small">Version {pkg.version} · build {buildVersion}</p>
			</div>
		</div>
		<p class="muted small">Your Lyrion Music Server, securely, from anywhere. Free software under the GNU GPL (version 2 or later).</p>
		<p class="small"><a class="link" href="https://github.com/JameZUK/lyrion-anywhere" target="_blank" rel="noopener noreferrer">Project page and source code</a></p>
	</div>

</section>

<style>
	.check {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin: 0.5rem 0;
	}
	.settings {
		display: grid;
		gap: 1.25rem;
		max-width: 760px;
	}
	.card h2 {
		margin-top: 0;
	}
	/* Developer diagnostics: last, and folded away. */
	.developer > summary {
		cursor: pointer;
		font-size: 1.2rem;
		font-weight: 700;
		min-height: 44px;
		display: flex;
		align-items: center;
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
	.choice.off {
		cursor: not-allowed;
		opacity: 0.5;
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
	.start-with {
		margin: 0.75rem 0;
	}
	.formats {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem 1.5rem;
		margin: 0.75rem 0;
	}
	.about-row {
		display: flex;
		align-items: center;
		gap: 1rem;
	}
	.about-row img {
		flex: none;
	}
	.about-name {
		margin: 0;
		font-weight: 700;
		font-size: 1.1rem;
	}
	.about-row .small {
		margin-top: 0.15rem;
	}
	.steps {
		margin: 0;
		padding-left: 1.3rem;
		display: grid;
		gap: 0.3rem;
	}
</style>
