<script lang="ts">
	// Settings › About › Advanced (Wave 3.5): this device's player (start, stop, restart), the
	// playback engine and the volume source (each with the reason for the automatic choice), and
	// the diagnostics for troubleshooting, folded away. The only place LMS is named by name.
	import { app } from '#lib/state/app.svelte';
	import { SERVER_TOO_OLD_TEXT, device } from '#lib/player/device.svelte';
	import { OUTDATED_TEXT } from '#lib/state/status';
	import { detect } from '#lib/player/capabilities';
	import { MODE_LABELS, chooseMode, detectEnvironment, loadPreference, savePreference, type ModePreference } from '#lib/player/mode';
	import { FIXED_LEVEL_EXPLANATION, FIXED_LEVEL_LABEL, FIXED_LEVEL_NO_FOLLOW, VOLUME_LABELS, canFollow, planVolume, type VolumeSetting } from '#lib/player/volume';
	import Icon from '#lib/components/Icon.svelte';
	import PickerRow from '#lib/components/PickerRow.svelte';
	import Switch from '#lib/components/Switch.svelte';

	// Built in: every browser is checked, and the best playback mode chosen automatically.
	const caps = detect();
	const env = detectEnvironment();
	let pref = $state<ModePreference>(loadPreference());
	const planned = $derived(chooseMode(caps, env, pref));
	const active = $derived(device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting');
	const effective = $derived(active && device.choice ? device.choice : planned);
	function setPref(p: ModePreference) {
		pref = p;
		savePreference(p);
	}

	// Volume on this device: follows LMS where the browser allows it, otherwise the device's
	// own buttons (see lib/player/volume.ts).
	const volAbility = $derived({ mode: active && device.choice ? device.choice.mode : planned.mode, mediaVolume: device.volumeAbility().mediaVolume });
	const volFollow = $derived(canFollow(volAbility));
	const volPlan = $derived(active && device.volumePlan ? device.volumePlan : planVolume(volAbility, device.volumeSetting));
	const VOLUME_CHOICES: VolumeSetting[] = ['auto', 'follow', 'device'];

	const mb = (b?: number) => (b ? `${(b / 1048576).toFixed(1)} MB` : '–');
	const stateText = $derived(
		{ off: 'Not running', starting: 'Starting…', connected: 'Ready', reconnecting: 'Reconnecting…', stopped: 'Stopped', error: 'Couldn’t start' }[device.state],
	);
</script>

<h2 class="sub-h">This device’s player</h2>
<p class="player-state" data-player-state>
	<span class="pill" class:on={active}>{stateText}</span>
	{#if active}
		<button class="btn" onclick={() => device.stop()} data-player-stop><Icon name="stop" size={18} />Stop the player</button>
		{#if device.choice && planned.mode !== device.choice.mode}
			<button class="btn" onclick={async () => { device.stop(); await device.start(); }} data-player-restart>Restart it to apply the engine</button>
		{/if}
	{:else}
		<button class="btn primary" onclick={() => device.start()} disabled={!device.supported || device.serverTooOld || device.outdated} data-player-start><Icon name="device" size={18} />Start the player</button>
	{/if}
</p>
{#if !device.supported}<p class="muted small">This browser can’t play music itself.</p>{/if}
{#if device.serverTooOld}<p class="small" role="status" data-server-too-old>{SERVER_TOO_OLD_TEXT}</p>{/if}
{#if device.outdated}<p class="small" role="status" data-outdated>{OUTDATED_TEXT}</p>{/if}

<div class="rows">
	<PickerRow
		label="Playback engine"
		value={pref}
		options={[
			{ value: 'auto', label: 'Automatic', note: `Now: ${MODE_LABELS[planned.mode]}` },
			{ value: 'worklet', label: 'Web Audio' },
			{ value: 'media', label: 'Media element' },
		]}
		onchange={(v) => setPref(v as ModePreference)}
		note={`${MODE_LABELS[effective.mode]}: ${effective.reason}`}
		id="playback-engine"
	/>
	<Switch checked={device.fixedLevel} onchange={(on) => device.setFixedLevel(on)} label={FIXED_LEVEL_LABEL} note={FIXED_LEVEL_EXPLANATION} data-fixed-level />
	<PickerRow
		label="Volume on this device"
		value={device.volumeSetting}
		options={VOLUME_CHOICES.map((v) => ({ value: v, label: VOLUME_LABELS[v], ...(v === 'follow' && !volFollow.ok ? { disabled: true, note: `Not available: ${volFollow.why}` } : {}) }))}
		onchange={(v) => device.setVolumeSetting(v as VolumeSetting)}
		note={device.fixedLevel ? FIXED_LEVEL_NO_FOLLOW : `${volPlan.follow ? 'Follows LMS' : 'Device buttons'}: ${volPlan.reason}`}
		disabled={device.fixedLevel}
		id="volume-source"
	/>
</div>

<details class="diagnostics" data-developer>
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

<style>
	.sub-h {
		margin: 0.5rem 0 0.5rem;
		font-size: 1rem;
	}
	.player-state {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
		margin: 0 0 0.75rem;
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
	.rows {
		display: grid;
		gap: 2px;
	}
	.small {
		font-size: 0.85rem;
	}
	.diagnostics {
		margin-top: 1.5rem;
	}
	.diagnostics > summary {
		cursor: pointer;
		font-weight: 650;
		min-height: 44px;
		display: flex;
		align-items: center;
	}
	h3 {
		font-size: 1rem;
		margin-top: 1.25rem;
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
</style>
