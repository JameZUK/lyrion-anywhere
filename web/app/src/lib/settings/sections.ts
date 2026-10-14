// Settings is an index (Wave 3.5, rule 5): a handful of rows, each opening a page of its own
// (`#/settings/<id>`) with a one-line summary of its current value. Advanced is under About.
// The same tree as the Android app's (This device is Android's own).

export type SectionId = 'account' | 'appearance' | 'playback' | 'now-playing' | 'downloads' | 'history' | 'install' | 'about' | 'advanced';

export interface Section {
	id: SectionId;
	label: string;
	icon: string;
	/** Under another section (Advanced is under About), not a row of the index. */
	parent?: SectionId;
}

export const SECTIONS: readonly Section[] = [
	{ id: 'account', label: 'Account', icon: 'artist' },
	{ id: 'appearance', label: 'Appearance', icon: 'art' },
	{ id: 'playback', label: 'Playback', icon: 'output' },
	{ id: 'now-playing', label: 'Now playing', icon: 'nowplaying' },
	{ id: 'downloads', label: 'Downloads', icon: 'download' },
	{ id: 'history', label: 'Listening history', icon: 'clock' },
	{ id: 'install', label: 'Install the app', icon: 'plus' },
	{ id: 'about', label: 'About', icon: 'info' },
	{ id: 'advanced', label: 'Advanced', icon: 'settings', parent: 'about' },
];

/** The section named in an address, or null for anything else. */
export function sectionOf(v: unknown): Section | null {
	return typeof v === 'string' ? (SECTIONS.find((s) => s.id === v) ?? null) : null;
}
