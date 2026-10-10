// The user's appearance choices: applied at start-up, saved in this browser, and re-applied
// when they change or when the system switches between light and dark.
import { applyTheme, loadPrefs, resolveMode, savePrefs, sanitise, type ThemePrefs } from './themes';

class Appearance {
	prefs = $state<ThemePrefs>(loadPrefs());
	/** What the page shows now ('auto' resolved). */
	mode = $state<'light' | 'dark'>('light');
	#media: MediaQueryList | null = null;

	#systemDark() {
		return this.#media?.matches ?? false;
	}

	#apply() {
		this.mode = resolveMode(this.prefs.mode, this.#systemDark());
		applyTheme(this.prefs, this.#systemDark());
	}

	/** Apply the saved choices and follow the system's light/dark setting. Call once. */
	start() {
		if (this.#media || typeof document === 'undefined') return;
		this.#media = globalThis.matchMedia?.('(prefers-color-scheme: dark)') ?? null;
		this.#media?.addEventListener('change', () => this.#apply());
		this.#apply();
	}

	set(change: Partial<ThemePrefs>) {
		this.prefs = sanitise({ ...this.prefs, ...change });
		savePrefs(this.prefs);
		this.#apply();
	}
}

export const appearance = new Appearance();
