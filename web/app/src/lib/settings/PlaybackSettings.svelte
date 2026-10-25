<script lang="ts">
	// Settings › Playback (Wave 3.5): which player the app starts on, this browser's streaming
	// quality, the sleep timer's fade, and this device's name as a player. The playback engine
	// and the volume source are under About › Advanced.
	import { app, session } from '#lib/state/app.svelte';
	import { device } from '#lib/player/device.svelte';
	import * as auth from '#lib/api/auth';
	import { LEVEL_LABELS, PREFERENCES, autoNote, connection, type QualityPreference } from '#lib/player/stream-quality';
	import { loadDefault, saveDefault, type DefaultSetting } from '#lib/player/default-player';
	import { isThisDevice } from '#lib/player/outputs';
	import { textOf } from '#lib/lms/format';
	import { FADE_CHOICES } from '#lib/lms/sleep';
	import PickerRow from '#lib/components/PickerRow.svelte';

	const QUALITY_LABELS: Record<string, string> = { auto: 'Automatic', original: 'Original', high: 'High', low: 'Data saver' };
	// Whether this browser can report its network (Chrome on Android can; Firefox and Safari
	// can't, so Automatic can only mean Original there and the note says so).
	const net = connection();
	const QUALITY_NOTES: Record<string, string> = {
		auto: autoNote(net),
		original: 'Each track as it is in your library',
		high: 'Lossless tracks re-encoded by the gateway to Opus 160 kb/s (about 70 MB an hour)',
		low: 'Lossless tracks re-encoded by the gateway to Opus 96 kb/s (about 45 MB an hour)',
	};
	const qualityNow = $derived(
		device.qualityOverride
			? `Now: ${LEVEL_LABELS[device.qualityOverride]}, overridden on the signal path. This setting applies again when the page reloads or the network changes.`
			: device.qualityPreference === 'auto'
			? `Now: ${QUALITY_LABELS[device.quality]}${net ? '' : " (this browser can't report its network)"}. Applies from the next track.`
			: 'Applies from the next track. The quality badge on Now playing shows what is streamed.',
	);

	// Which player the app uses when it opens on this device (SPEC D20), per user.
	const user = $derived(session.current?.user.username ?? '');
	let startWith = $state<DefaultSetting>({ kind: 'device' });
	$effect(() => {
		startWith = loadDefault(user);
	});
	const startKey = (d: DefaultSetting) => (d.kind === 'player' ? d.id : d.kind);
	const startPlayers = $derived(app.players.filter((p) => !isThisDevice(p.playerid)));
	const startOptions = $derived([
		{ value: 'device', label: 'This device', ...(device.supported ? {} : { note: "This browser can't play music itself", disabled: true }) },
		{ value: 'last', label: 'The player used last' },
		...startPlayers.map((p) => ({ value: p.playerid, label: textOf(p.name) || 'A player' })),
		...(startWith.kind === 'player' && !startPlayers.some((p) => startWith.kind === 'player' && p.playerid === startWith.id) ? [{ value: startKey(startWith), label: 'A player not connected now' }] : []),
	]);
	function setStartWith(v: string) {
		const d: DefaultSetting = v === 'device' || v === 'last' ? { kind: v } : { kind: 'player', id: v };
		startWith = d;
		saveDefault(user, d);
	}

	// This device's name: the player name in LMS. Renaming renames that player too.
	const deviceName = $derived(app.session?.device.name ?? '');
	let newName = $state('');
	let msg = $state('');
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
	const fadeLabel = (f: number) => (f === 0 ? 'Off' : f < 60 ? `${f} seconds` : f === 60 ? '1 minute' : `${f / 60} minutes`);
</script>

<div class="rows">
	<PickerRow label="When the app opens, play on" value={startKey(startWith)} options={startOptions} onchange={setStartWith} id="start-with" />
	<PickerRow
		label="Streaming quality on this device"
		value={device.qualityPreference}
		options={PREFERENCES.map((p) => ({ value: p, label: QUALITY_LABELS[p] ?? p, note: QUALITY_NOTES[p] }))}
		onchange={(v) => device.setQualityPreference(v as QualityPreference)}
		note={qualityNow}
		id="stream-quality"
	/>
	<PickerRow
		label="Sleep timer fade"
		value={String(app.sleepFade)}
		options={FADE_CHOICES.map((f) => ({ value: String(f), label: fadeLabel(f) }))}
		onchange={(v) => app.setSleepFade(Number(v))}
		note="How long this device fades out before it stops (other players follow your music server)."
		id="sleep-fade"
	/>
</div>

<h2 class="sub-h">This device’s player name</h2>
<form onsubmit={renameThis} class="inline" data-rename>
	<label class="field"><span>Name</span><input bind:value={newName} maxlength="64" autocomplete="off" spellcheck="false" required data-device-name /></label>
	<button class="btn" type="submit" disabled={!auth.validDeviceName(newName) || newName.trim() === deviceName}>Rename</button>
</form>
<p class="note" role="status" data-rename-status>{msg}</p>

<style>
	.rows {
		display: grid;
		gap: 2px;
	}
	.sub-h {
		margin: 1.5rem 0 0.25rem;
		font-size: 1rem;
	}
	.inline {
		display: flex;
		flex-wrap: wrap;
		align-items: end;
		gap: 0.6rem;
	}
	.inline .field {
		flex: 1 1 14rem;
	}
	.note:empty {
		display: none;
	}
</style>
