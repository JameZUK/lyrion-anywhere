// The transport controls every surface uses (the player bar, Now playing, the queue views,
// the lock screen, keyboard shortcuts): which of them work now, and what each one does. With
// the server in reach they send LMS commands, as always; out of reach (or busy) they act on
// this device when it is the player (./local.svelte) and are disabled otherwise. While this
// device plays its downloads (the offline player's queue is active), they act on that engine
// instead (Wave 3.5 item 14: two engines, one face; see ./now.svelte).
import { api, app } from '$lib/state/app.svelte';
import * as cmd from '$lib/lms/commands';
import { connectionMessage, explain, needsServer } from '$lib/state/reachability';
import { downloads, offlinePlayer, offlineStore } from '$lib/offline/index.svelte';
import type { StoredTrack } from '$lib/offline/types';
import { device } from './device.svelte';
import { gates, type Gates, type LocalKind } from './local-plan';
import { LocalPlayback } from './local.svelte';
import { describePageCommand } from './diagnostics';

export const local = new LocalPlayback({
	device: {
		mac: () => device.mac,
		output: () => String(device.detail.output ?? ''),
		held: () => device.held,
		received: () => device.detail.received === true,
		track: () => (typeof device.detail.track === 'number' ? device.detail.track : null),
		fresh: () => device.freshSession,
		local: (m) => device.local(m),
		note: (m) => device.note(m),
	},
	status: () => app.now,
	apply: (p) => app.applyLocal(p),
	elapsed: () => app.elapsed,
	downloaded: (id) => downloads.savedById(id),
	audio: (t) => offlineStore.getAudio(t.user, t.key),
	rpc: (player, c) => api.rpc(player, c),
	refreshed: () => app.soon(),
	volume: () => device.gain,
});

/** What works while downloads play: the transport and seeking; nothing LMS does. */
const DOWNLOADS_GATES = (index: number, count: number, duration: number): Gates => ({
	playPause: true,
	next: index < count - 1,
	previous: count > 0,
	seek: duration > 0,
	volume: false,
	other: false,
	jump: (i) => i >= 0 && i < count,
});

class Controls {
	/** This device plays its downloads: the transport acts on the downloads engine. */
	get downloads(): boolean {
		return offlinePlayer.active;
	}

	/** Playing now (the downloads engine's state while it is active). */
	get playing(): boolean {
		return this.downloads ? offlinePlayer.playing : app.playing;
	}

	/** This device is the player being controlled. */
	get thisDevice(): boolean {
		return !!device.mac && app.playerId === device.mac;
	}

	/** Out of reach (or busy) and playing on by itself. */
	get local(): boolean {
		return local.engaged;
	}

	/** Which controls work now. */
	get gates(): Gates {
		if (this.downloads) return DOWNLOADS_GATES(offlinePlayer.index, offlinePlayer.queue.length, offlinePlayer.duration);
		const s = app.now;
		const engaged = local.engaged;
		void local.version;
		return gates({
			reach: app.reach,
			hasNow: !!s,
			thisDevice: this.thisDevice && engaged,
			queue: {
				index: engaged ? local.index : Number(s?.playlist_cur_index ?? 0),
				count: Number(s?.playlist_tracks ?? 0) || 0,
				repeat: Number(s?.['playlist repeat'] ?? 0) || 0,
			},
			elapsed: app.elapsed,
			available: local.available,
			fromStart: local.fromStart,
			source: local.source,
			streamControllable: device.held || String(device.detail.output ?? '') === 'Playing',
		});
	}

	/** The one connection message ('' while in reach). */
	get message(): string {
		return connectionMessage(app.reach, local.engaged && local.source !== null && !local.paused, local.engaged ? local.waiting : '');
	}

	/**
	 * Attributes for a control that needs the server (Play on a library page, Download, LMS's
	 * menus, moving playback…): disabled while it is out of reach or busy, as `app.canQueue`,
	 * and pointing at the connection message. `ok`: anything else the control needs.
	 */
	needsServer(ok = true) {
		return needsServer(app.reach, this.message, ok);
	}

	/** Why a control is `disabled`, when the connection is the reason (title and description). */
	why(disabled: boolean) {
		return explain(disabled, app.reach, this.message);
	}

	/**
	 * The queue mark for entry `index` (LMS track `id`). One mark per meaning (Wave 3.5 rule 2):
	 * "Downloaded" whenever the track is; "On this device" (the cache) only while the server is
	 * out of reach, when it changes what can play.
	 */
	mark(index: number, id: unknown): LocalKind | null {
		const m = local.mark(index, id, this.thisDevice, Number(app.now?.playlist_cur_index ?? -1));
		return m === 'device' && app.reach === 'online' ? null : m;
	}

	toggle() {
		if (this.downloads) return offlinePlayer.toggle();
		if (local.engaged) return local.toggle();
		void app.send(app.playing ? cmd.pause(true) : cmd.play());
	}

	/** Play (the lock screen's own Play): resume if paused here, else tell LMS. */
	play() {
		if (this.downloads) return offlinePlayer.playing ? undefined : offlinePlayer.toggle();
		if (local.engaged) return local.paused || local.source === null ? local.toggle() : undefined;
		void app.send(cmd.play());
	}

	pause() {
		if (this.downloads) return offlinePlayer.pause();
		if (local.engaged) return local.paused ? undefined : local.toggle();
		void app.send(cmd.pause(true));
	}

	next() {
		if (this.downloads) return void offlinePlayer.next();
		if (local.engaged) return local.next();
		void app.send(cmd.skip(1));
	}

	previous() {
		if (this.downloads) return void offlinePlayer.previous();
		if (local.engaged) return local.previous();
		void app.send(cmd.skip(-1));
	}

	/** Play queue entry `i`. */
	jump(i: number) {
		if (this.downloads) return void offlinePlayer.jump(i);
		if (local.engaged) return local.jump(i);
		void app.send(cmd.jump(i));
	}

	seek(seconds: number): Promise<void> {
		if (this.downloads) {
			offlinePlayer.seek(seconds);
			return Promise.resolve();
		}
		if (local.engaged) {
			local.seek(seconds);
			return Promise.resolve();
		}
		return app.seek(seconds);
	}

	/**
	 * Play downloads on this device (issue #11). Two engines must never sound at once: whatever
	 * this device is already playing is paused first, through LMS while the server is in reach
	 * and held here otherwise, so what is heard is always what the screens show and control.
	 */
	async playDownloads(user: string, tracks: StoredTrack[], start = 0): Promise<void> {
		this.quietStream();
		await offlinePlayer.playList(user, tracks, start);
	}

	/** Pause what this device plays on the server's behalf, if anything is audible. */
	quietStream() {
		// Out of reach and playing on by itself (its stream, or a download from LMS's queue).
		if (local.engaged && !local.paused && local.source !== null) {
			local.toggle();
			return;
		}
		if (!device.mac || String(device.detail.output ?? '') !== 'Playing' || device.held) return;
		if (app.reach === 'online') {
			// The usual pause when this device is the player being controlled; else straight to
			// this device's own player, whichever one the screens show.
			device.note(describePageCommand(cmd.pause(true), 'downloads start playing here'));
			if (this.thisDevice) void app.send(cmd.pause(true));
			else void api.rpc(device.mac, cmd.pause(true)).catch(() => {});
			return;
		}
		device.local({ hold: true });
	}
}

export const controls = new Controls();

// The server's music starting on this device ends the downloads: one engine at a time, and the
// transport goes back to LMS (issue #11). Pausing the stream to start downloads is no start.
device.onPlaying(() => {
	if (offlinePlayer.active) offlinePlayer.stop();
});
