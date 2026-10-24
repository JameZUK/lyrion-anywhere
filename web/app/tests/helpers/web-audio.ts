// Web Audio as capability detection sees it (src/lib/player/capabilities.ts): the
// AudioWorkletNode constructor and an `audioWorklet` on the AudioContext prototype.
import { vi } from 'vitest';

/** An AudioContext whose prototype has `audioWorklet`, as in a secure context. */
export class WorkletContext {
	get audioWorklet() {
		return { addModule: async () => {} };
	}
}

/** Make this "browser" able (or unable) to run Web Audio worklets. */
export function stubWebAudio(yes = true) {
	vi.stubGlobal('AudioWorkletNode', yes ? class {} : undefined);
	vi.stubGlobal('AudioContext', yes ? WorkletContext : undefined);
}
