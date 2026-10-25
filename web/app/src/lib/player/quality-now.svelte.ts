// The quality badge and the signal-path diagram for what is playing now
// (docs/plans/signal-path-redesign.md): this device's player (the stream heard, from the
// worker and the gateway's stream message), another player (only the file is known), or this
// browser's downloads. Every screen that shows the badge reads it here, so they all agree.
import { app } from '#lib/state/app.svelte';
import { offlinePlayer } from '#lib/offline/index.svelte';
import { device } from './device.svelte';
import { describe, isLowered, type Diagram } from './quality-path';
import { qualityInputs, type PathFacts } from './quality-now';
import { connection, originOf } from './stream-quality';

class QualityNow {
	/** This device is the selected player, and its player is running. */
	get here(): boolean {
		return !offlinePlayer.active && !!app.playerId && app.playerId === device.mac && device.state === 'connected';
	}

	/** The current track can be sought (so a change of level can take effect now). */
	get seekable(): boolean {
		return Number(app.now?.duration ?? 0) > 0 && app.now?.can_seek !== 0;
	}

	get facts(): PathFacts {
		if (offlinePlayer.active) {
			const t = offlinePlayer.current;
			return { player: 'offline', download: t ? { format: t.format, codec: t.codec, ...(t.source ? { source: t.source } : {}) } : null };
		}
		const track = app.now?.playlist_loop?.[0] ?? null;
		if (!this.here) return { player: 'remote', playerName: app.nameOf(app.playerId), track };
		const d = device.detail;
		return {
			player: 'local',
			track,
			source: d.source,
			served: d.served,
			sent: d.sent,
			engineRate: Number(d.sample_rate) > 0 ? Number(d.sample_rate) : null,
			deviceRate: device.deviceRate(),
			volumeScaled: device.volumePlan?.follow === true && device.gain < 0.999,
			origin: originOf(device.qualityPreference, device.qualityOverride, connection()),
			seekable: this.seekable,
		};
	}

	/** The badge, the blocks and the switch. */
	get diagram(): Diagram {
		return describe(qualityInputs(this.facts));
	}
}

export const qualityNow = new QualityNow();

/** The badge's tint: Transcoded and Lossy in amber (the word always carries the meaning). */
export const amber = (d: Diagram): boolean => !!d.badge && isLowered(d.badge.verdict);
