// Now playing's per-device choices, shared by Now playing and Settings (see prefs.ts).
import { loadNowPlaying, saveNowPlaying, type Layout } from './prefs';
import type { VisualStyle } from '#lib/visual/draw';

class NowPlayingState {
	layout = $state<Layout>('art');
	visualiser = $state(false);
	style = $state<VisualStyle>('bars');
	queuePanel = $state(true);

	constructor() {
		this.reload();
	}

	/** Read the stored choices again. */
	reload() {
		const p = loadNowPlaying();
		this.layout = p.layout;
		this.visualiser = p.visualiser;
		this.style = p.style;
		this.queuePanel = p.queuePanel;
	}

	#save() {
		saveNowPlaying({ layout: this.layout, visualiser: this.visualiser, style: this.style, queuePanel: this.queuePanel });
	}

	setLayout(l: Layout) {
		this.layout = l;
		this.#save();
	}
	setVisualiser(on: boolean) {
		this.visualiser = on;
		this.#save();
	}
	setStyle(s: VisualStyle) {
		this.style = s;
		this.#save();
	}
	setQueuePanel(on: boolean) {
		this.queuePanel = on;
		this.#save();
	}
}

export const nowPlaying = new NowPlayingState();
