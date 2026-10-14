// The transport controls every surface uses (the player bar, Now playing, the queue views,
// the lock screen, keyboard shortcuts): which of them work now, and what each one does. With
// the server in reach they send LMS commands, as always; out of reach (or busy) they act on
// this device when it is the player (./local.svelte) and are disabled otherwise.
import { api, app } from '$lib/state/app.svelte';
import * as cmd from '$lib/lms/commands';
import { connectionMessage, explain, needsServer } from '$lib/state/reachability';
import { downloads, offlineStore } from '$lib/offline/index.svelte';
import { device } from './device.svelte';
import { gates, type Gates, type LocalKind } from './local-plan';
import { LocalPlayback } from './local.svelte';

export const local = new LocalPlayback({
	device: {
		mac: () => device.mac,
		output: () => String(device.detail.output ?? ''),
		held: () => device.held,
		received: () => device.detail.received === true,
		track: () => (typeof device.detail.track === 'number' ? device.detail.track : null),
		fresh: () => device.freshSession,
		local: (m) => device.local(m),
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

class Controls {
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

	/** The queue mark for entry `index` (LMS track `id`). */
	mark(index: number, id: unknown): LocalKind | null {
		return local.mark(index, id, this.thisDevice, Number(app.now?.playlist_cur_index ?? -1));
	}

	toggle() {
		if (local.engaged) return local.toggle();
		void app.send(app.playing ? cmd.pause(true) : cmd.play());
	}

	/** Play (the lock screen's own Play): resume if paused here, else tell LMS. */
	play() {
		if (local.engaged) return local.paused || local.source === null ? local.toggle() : undefined;
		void app.send(cmd.play());
	}

	pause() {
		if (local.engaged) return local.paused ? undefined : local.toggle();
		void app.send(cmd.pause(true));
	}

	next() {
		if (local.engaged) return local.next();
		void app.send(cmd.skip(1));
	}

	previous() {
		if (local.engaged) return local.previous();
		void app.send(cmd.skip(-1));
	}

	/** Play queue entry `i`. */
	jump(i: number) {
		if (local.engaged) return local.jump(i);
		void app.send(cmd.jump(i));
	}

	seek(seconds: number): Promise<void> {
		if (local.engaged) {
			local.seek(seconds);
			return Promise.resolve();
		}
		return app.seek(seconds);
	}
}

export const controls = new Controls();
