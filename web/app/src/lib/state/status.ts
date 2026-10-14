// The one status slot (Wave 3.5, rule 1): lasting states share one line at the top of every
// page, the most important first. Moving playback > out of reach > busy > reconnecting; then
// the two offers about this device's own player (the sound held back until a tap, and "it was
// playing before the page reloaded"). Results ("Saved", "Couldn't…") are snackbars instead,
// and "Continue on…" is a snackbar with an action. Nothing else stacks.
import type { Reachability } from './reachability';

export type StatusKind = 'moving' | 'unreachable' | 'busy' | 'reconnecting' | 'held' | 'resume';

export interface StatusInput {
	/** The player playback is moving to ('' when not moving). */
	moving: string;
	reach: Reachability;
	/** The connection message ('' while in reach). */
	message: string;
	/** This device is the player and its own connection is being made again. */
	reconnecting: boolean;
	/** This device plays, but the browser holds the sound back until a tap. */
	heldBack: boolean;
	/** This device was playing before the page reloaded, and its player is not running now. */
	resume: boolean;
}

export interface Status {
	kind: StatusKind;
	text: string;
}

export const RECONNECTING_TEXT = 'Reconnecting this device…';
export const HELD_TEXT = 'Playing on this device. Tap anywhere to turn the sound on.';
export const RESUME_TEXT = 'This device was playing before the page reloaded.';

/** What the slot shows now, or null for nothing. */
export function statusOf(s: StatusInput): Status | null {
	if (s.moving) return { kind: 'moving', text: `Moving playback to ${s.moving}…` };
	if (s.reach === 'unreachable' && s.message) return { kind: 'unreachable', text: s.message };
	if (s.reach === 'busy' && s.message) return { kind: 'busy', text: s.message };
	if (s.reconnecting) return { kind: 'reconnecting', text: RECONNECTING_TEXT };
	if (s.heldBack) return { kind: 'held', text: HELD_TEXT };
	if (s.resume) return { kind: 'resume', text: RESUME_TEXT };
	return null;
}
