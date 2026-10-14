// Names for saving the queue as a playlist: the same rules the gateway applies (LMS writes
// the playlist to a file of that name), checked here first so the dialog can say what's
// wrong as the name is typed, and a suggested name to start from.

/** Longest name, in characters. */
export const PLAYLIST_NAME_MAX = 100;

/** Characters LMS or common filesystems won't take in a file name. */
const RESERVED = /[/\\:*?"<>|]/;
/** Control characters, and invisible ones that could disguise a name (code point ranges). */
const HIDDEN_RANGES: readonly [number, number][] = [
	[0x0000, 0x001f],
	[0x007f, 0x009f],
	[0x200b, 0x200f],
	[0x2028, 0x202e],
	[0x2060, 0x206f],
	[0xfeff, 0xfeff],
	[0xfff9, 0xfffb],
];
const hidden = (c: string) => {
	const n = c.codePointAt(0) ?? 0;
	return HIDDEN_RANGES.some(([a, b]) => n >= a && n <= b);
};
/** Windows device names: no file of that name (with any extension) can be made there. */
const DEVICE = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;

/** The name as it will be sent: composed (NFC) and trimmed. */
export function cleanPlaylistName(s: string): string {
	return String(s ?? '')
		.normalize('NFC')
		.trim();
}

/** Why `name` (already cleaned) can't be used, or '' when it can. */
export function playlistNameProblem(name: string): string {
	if (!name) return 'Give the playlist a name.';
	if ([...name].length > PLAYLIST_NAME_MAX) return `Names are at most ${PLAYLIST_NAME_MAX} characters.`;
	if ([...name].some(hidden)) return 'Names can’t contain control or invisible characters.';
	if (RESERVED.test(name)) return 'Names can’t contain / \\ : * ? " < > or |.';
	if (name.includes('..')) return 'Names can’t contain “..”.';
	if (name.startsWith('.')) return 'Names can’t start with a dot.';
	if (name.endsWith('.')) return 'Names can’t end with a dot.';
	if (DEVICE.test(name.split('.')[0]!.trimEnd())) return 'That name is reserved.';
	return '';
}

/** `s` made into an acceptable name, as far as that's possible ('' if nothing is left). */
function tidy(s: string): string {
	let out = cleanPlaylistName(
		[...s]
			.filter((c) => !hidden(c))
			.join('')
			.replace(new RegExp(RESERVED.source, 'g'), ' ')
			.replace(/\.{2,}/g, '.')
			.replace(/\s+/g, ' '),
	);
	out = [...out].slice(0, PLAYLIST_NAME_MAX).join('');
	out = out.replace(/^[.\s]+|[.\s]+$/g, '');
	return playlistNameProblem(out) ? '' : out;
}

/**
 * A name to start from: the album, when the queue is one album; otherwise "Queue" and
 * today's date ("Queue 14 October 2026").
 */
export function suggestPlaylistName(tracks: readonly { album?: unknown }[], now = new Date()): string {
	const albums = new Set(tracks.map((t) => (typeof t.album === 'string' ? t.album.trim() : '')));
	if (albums.size === 1) {
		const album = tidy([...albums][0]!);
		if (album) return album;
	}
	const day = now.toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric' });
	return tidy(`Queue ${day}`) || 'Queue';
}
