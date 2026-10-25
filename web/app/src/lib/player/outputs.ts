// Where the music plays (SPEC D20): this device by default; another player on request, by
// controlling it or by moving playback there (Transfer, docs/PROTOCOL.md).
import { app } from '#lib/state/app.svelte';
import { SERVER_TOO_OLD_TEXT } from '#lib/state/status';
import { device } from './device.svelte';

/** This device's player is running, or on its way. */
export function deviceRunning(): boolean {
	return device.state === 'connected' || device.state === 'reconnecting' || device.state === 'starting';
}

/** Is `id` this device's own player? */
export function isThisDevice(id: string): boolean {
	return !!id && (id === app.deviceMac || id === device.mac);
}

/**
 * The server is too old for this browser's player: say so, and do nothing else (no waiting
 * for this device, no switch to another player). True when refused.
 */
function refusedTooOld(): boolean {
	if (!device.serverTooOld) return false;
	app.notice = SERVER_TOO_OLD_TEXT;
	return true;
}

/**
 * Control this device, starting its player if needed. Call it from a tap (sound needs one).
 * Refused, saying why, when the server is too old for it (known before, or learnt while
 * starting: then the app stops waiting for this device and stays with the player it had).
 */
export async function useThisDevice(): Promise<void> {
	if (!device.supported || refusedTooOld()) return;
	app.preferDevice();
	if (deviceRunning()) return;
	await device.start();
	if (device.serverTooOld) {
		app.deviceFailed();
		refusedTooOld();
	}
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
	if (refusedTooOld()) return false;
	// Inside the tap: browsers only let sound start in one.
	if (!deviceRunning()) await device.start();
	// Learnt while starting: playback stays where it is.
	if (refusedTooOld()) return false;
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
