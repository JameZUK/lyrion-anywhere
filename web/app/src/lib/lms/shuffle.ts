// Album-aware shuffle (Wave 2 §3.5): LMS's shuffle modes 0 (off), 1 (tracks) and 2 (albums).
// The button cycles Off → Tracks → Albums, and says which.

export type ShuffleMode = 0 | 1 | 2;
export const SHUFFLE_MODES: readonly ShuffleMode[] = [0, 1, 2];
export const SHUFFLE_NAMES: Record<ShuffleMode, string> = { 0: 'Off', 1: 'Tracks', 2: 'Albums' };

/** LMS's `playlist shuffle` value as a mode (anything odd counts as off). */
export function shuffleMode(v: unknown): ShuffleMode {
	const n = typeof v === 'number' ? v : typeof v === 'string' && /^[0-2]$/.test(v.trim()) ? Number(v) : 0;
	return n === 1 || n === 2 ? n : 0;
}

/** The mode after `m` when the button is pressed. */
export function nextShuffle(m: ShuffleMode): ShuffleMode {
	return ((m + 1) % 3) as ShuffleMode;
}

/** What the button is called (its state included), e.g. "Shuffle: albums". */
export function shuffleLabel(m: ShuffleMode): string {
	return `Shuffle: ${SHUFFLE_NAMES[m].toLowerCase()}`;
}
