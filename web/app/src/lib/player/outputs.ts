// Where the music plays (SPEC D20): this device by default; another player on request, by
// controlling it or by moving playback there (Transfer, docs/PROTOCOL.md).
import { app } from '$lib/state/app.svelte';
import { device } from './device.svelte';

/** This device's player is running, or on its way. */
export function deviceRunning(): boolean {
	return device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting';
}

/** Is `id` this device's own player? */
export function isThisDevice(id: string): boolean {
	return !!id && (id === app.deviceMac || id === device.mac);
}

/** Control this device, starting its player if needed. Call it from a tap (sound needs one). */
export function useThisDevice() {
	if (!device.supported) return;
	app.preferDevice();
	if (!deviceRunning()) void device.start();
}

/** Move playback from the player in use to `target`, and control `target`. */
export async function moveTo(target: string): Promise<boolean> {
	const source = app.playerId;
	if (!source || source === target) return false;
	return app.moveTo(source, target, app.nameOf(target));
}

/** Bring playback from the player in use to this device. Call it from a tap. */
export async function bringHere(): Promise<boolean> {
	const source = app.playerId;
	const mac = app.deviceMac;
	if (!source || !mac || isThisDevice(source) || !device.supported) return false;
	// Inside the tap: browsers only let sound start in one.
	if (!deviceRunning()) await device.start();
	app.moving = 'this device';
	const listed = app.players.some((p) => p.playerid === mac) || (await app.waitForPlayer(mac));
	app.moving = '';
	if (!listed) {
		app.select(source);
		app.notice = `This device couldn't start playing, so playback stayed on ${app.nameOf(source) || 'the other player'}.`;
		return false;
	}
	return app.moveTo(source, mac, 'this device');
}
