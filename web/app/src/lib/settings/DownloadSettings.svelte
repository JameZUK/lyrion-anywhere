<script lang="ts">
	// Settings › Downloads (SPEC D5, §8.2, §8.3; Wave 3.5): the preferences (the usual format on
	// Wi-Fi and on mobile data, Wi-Fi only, a storage limit). The downloads themselves, and
	// removing them, are on the Downloads page (Manage downloads ›).
	import { downloads } from '$lib/offline/index.svelte';
	import { CAP_CHOICES, connectionKind, formatBytes, loadPrefs, networkType, reportsNetwork, requestPersist, savePrefs, storageInfo, type DownloadPrefs, type StorageInfo } from '$lib/offline/settings';
	import { FORMATS, FORMAT_LABELS } from '$lib/offline/types';
	import Icon from '$lib/components/Icon.svelte';
	import PickerRow from '$lib/components/PickerRow.svelte';
	import Switch from '$lib/components/Switch.svelte';

	let prefs = $state<DownloadPrefs>(loadPrefs());
	function setPrefs(p: Partial<DownloadPrefs>) {
		prefs = { ...prefs, ...p };
		savePrefs(prefs);
		// Wi-Fi only may hold or release the queue at once.
		downloads.networkChanged();
	}
	const network = networkType();
	const reports = reportsNetwork();
	const onCellular = connectionKind() === 'cellular';
	let storage = $state<StorageInfo>({});
	$effect(() => {
		void downloads.saved.length;
		let live = true;
		storageInfo().then((i) => live && (storage = i));
		return () => (live = false);
	});
	async function keepDownloads() {
		await requestPersist();
		storage = await storageInfo();
	}
	const FORMAT_NOTES: Record<string, string> = { original: 'The file as it is in your library (largest)', mp3: 'Plays everywhere', opus: 'About 1 MB a minute' };
	const formats = FORMATS.map((f) => ({ value: f, label: FORMAT_LABELS[f], note: FORMAT_NOTES[f] }));
	const caps = $derived([
		...CAP_CHOICES.map((c) => ({ value: String(c), label: c ? formatBytes(c) : 'No limit' })),
		...(prefs.capBytes && !(CAP_CHOICES as readonly number[]).includes(prefs.capBytes) ? [{ value: String(prefs.capBytes), label: formatBytes(prefs.capBytes) }] : []),
	]);
</script>

<div class="rows">
	<PickerRow label={`Format on Wi-Fi${network === 'wifi' ? ' (now)' : ''}`} value={prefs.wifi} options={formats} onchange={(v) => setPrefs({ wifi: v as DownloadPrefs['wifi'] })} note="If your music server can’t convert, Original is used." id="format-wifi" />
	<PickerRow label={`Format on mobile data${network === 'cellular' ? ' (now)' : ''}`} value={prefs.cellular} options={formats} onchange={(v) => setPrefs({ cellular: v as DownloadPrefs['cellular'] })} id="format-cellular" />
	<Switch
		checked={prefs.wifiOnly}
		onchange={(v) => setPrefs({ wifiOnly: v })}
		label="Download on Wi-Fi only"
		note={!reports ? 'This browser doesn’t say which network it is on, so downloads always start here.' : onCellular && prefs.wifiOnly ? 'You’re on mobile data now: downloads wait (Download now starts one anyway).' : 'On mobile data, downloads wait for Wi-Fi.'}
		data-wifi-only
		data-testid="wifi-only-note"
	/>
	<PickerRow label="Storage limit for downloads" value={String(prefs.capBytes)} options={caps} onchange={(v) => setPrefs({ capBytes: Number(v) })} note="A download that would go past it isn’t started." id="storage-limit" />
</div>

<p class="usage" data-testid="download-usage">
	<strong>{downloads.saved.length} downloaded {downloads.saved.length === 1 ? 'track' : 'tracks'}, {formatBytes(downloads.usedBytes)}</strong>
	{#if prefs.capBytes}<span class="muted"> of your {formatBytes(prefs.capBytes)} limit</span>{/if}
	{#if storage.quota}<span class="muted"> · this app uses {formatBytes(storage.usage ?? 0)} of {formatBytes(storage.quota)} the browser allows</span>{/if}
</p>
<p class="muted small" data-testid="persist-status">
	{storage.persisted === true ? 'The browser has agreed to keep downloads.' : storage.persisted === false ? 'The browser may clear downloads if the device runs short of space.' : 'This browser doesn’t say whether it keeps downloads.'}
	{#if storage.persisted === false}<button class="link-btn" onclick={keepDownloads}>Ask it to keep them</button>{/if}
</p>
<a class="manage" href="#/downloads" data-manage-downloads><Icon name="download" /><span class="grow">Manage downloads</span><span class="chev" aria-hidden="true">›</span></a>

<style>
	.rows {
		display: grid;
		gap: 2px;
	}
	.usage {
		margin: 1rem 0 0.25rem;
	}
	.small {
		font-size: 0.85rem;
		margin: 0;
	}
	.link-btn {
		border: 0;
		background: none;
		padding: 0.5rem 0.25rem;
		min-height: 44px;
		color: var(--accent);
		font: inherit;
		text-decoration: underline;
		cursor: pointer;
	}
	.manage {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 52px;
		margin-top: 0.75rem;
		padding: 0.5rem;
		border-radius: var(--radius-sm);
	}
	.manage:hover {
		background: var(--bg-sunken);
	}
	.manage :global(svg) {
		color: var(--fg-muted);
	}
	.grow {
		flex: 1;
		font-weight: 550;
	}
	.chev {
		color: var(--fg-muted);
		font-size: 1.2rem;
	}
</style>
