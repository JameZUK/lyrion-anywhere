// "Install the app": Android/desktop Chromium offer a prompt the page can trigger; iPhone and
// iPad need the Share menu, so the app explains how. Nothing shows once installed.

interface InstallPrompt extends Event {
	prompt(): Promise<void>;
	userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

export type InstallWay = 'installed' | 'prompt' | 'ios' | 'none';

/** How this browser can install the app (pure, for tests). */
export function installWay(o: { standalone: boolean; hasPrompt: boolean; userAgent: string; touchPoints: number }): InstallWay {
	if (o.standalone) return 'installed';
	if (o.hasPrompt) return 'prompt';
	const ios = /iPhone|iPad|iPod/.test(o.userAgent) || (/Macintosh/.test(o.userAgent) && o.touchPoints > 1); // iPadOS says "Macintosh"
	// On iOS every browser installs through the Share menu (Safari, and others since iOS 16.4).
	if (ios) return 'ios';
	return 'none';
}

class Install {
	#prompt: InstallPrompt | null = null;
	way = $state<InstallWay>('none');

	#update() {
		const standalone =
			globalThis.matchMedia?.('(display-mode: standalone)').matches === true ||
			(globalThis.navigator as Navigator & { standalone?: boolean })?.standalone === true;
		this.way = installWay({
			standalone,
			hasPrompt: !!this.#prompt,
			userAgent: globalThis.navigator?.userAgent ?? '',
			touchPoints: globalThis.navigator?.maxTouchPoints ?? 0,
		});
	}

	start() {
		if (typeof window === 'undefined') return;
		window.addEventListener('beforeinstallprompt', (e) => {
			e.preventDefault(); // offer it in Settings instead of a browser banner
			this.#prompt = e as InstallPrompt;
			this.#update();
		});
		window.addEventListener('appinstalled', () => {
			this.#prompt = null;
			this.way = 'installed';
		});
		this.#update();
	}

	async install(): Promise<boolean> {
		const p = this.#prompt;
		if (!p) return false;
		await p.prompt();
		const { outcome } = await p.userChoice;
		this.#prompt = null;
		this.#update();
		return outcome === 'accepted';
	}
}

export const install = new Install();
