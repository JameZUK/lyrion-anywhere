// Names for saving the queue as a playlist: the same rules the gateway applies (LMS writes
// the playlist to a file of that name), checked here first so the dialog can say what's
// wrong as the name is typed, and a suggested name to start from.

/** Longest name, in characters. */
export const PLAYLIST_NAME_MAX = 100;

/** Characters LMS or common filesystems won't take in a file name. */
const RESERVED = /[/\\:*?"<>|]/;
/**
 * Characters that draw nothing and could disguise a name: controls, separators, format
 * characters, fillers and variation selectors (code point ranges; the core's
 * `names::invisible`, which the gateway enforces).
 */
const HIDDEN_RANGES: readonly [number, number][] = [
	[0x0000, 0x001f],
	[0x007f, 0x009f],
	[0x00ad, 0x00ad],
	[0x034f, 0x034f],
	[0x0600, 0x0605],
	[0x061c, 0x061c],
	[0x06dd, 0x06dd],
	[0x070f, 0x070f],
	[0x0890, 0x0891],
	[0x08e2, 0x08e2],
	[0x115f, 0x1160],
	[0x180e, 0x180e],
	[0x200b, 0x200f],
	[0x2028, 0x202e],
	[0x2060, 0x206f],
	[0x3164, 0x3164],
	[0xfe00, 0xfe0f],
	[0xfeff, 0xfeff],
	[0xffa0, 0xffa0],
	[0xfff9, 0xfffb],
	[0x110bd, 0x110bd],
	[0x110cd, 0x110cd],
	[0x13430, 0x1343f],
	[0x1bca0, 0x1bca3],
	[0x1d173, 0x1d17a],
	[0xe0000, 0xe007f],
	[0xe0100, 0xe01ef],
];
const invisible = (c: string) => {
	const n = c.codePointAt(0) ?? 0;
	return HIDDEN_RANGES.some(([a, b]) => n >= a && n <= b);
};
/** The emoji presentation selector: "❤️" and many other emoji are a symbol followed by it. */
const EMOJI_SELECTOR = '\ufe0f';
/**
 * Whether the character at `i` of `chars` (code points) may not be in a name: an invisible
 * one, except the emoji selector straight after a symbol (not a letter, digit, space or
 * another invisible character), where it only chooses the symbol's emoji form.
 */
const hiddenAt = (chars: readonly string[], i: number) => {
	const c = chars[i]!;
	if (!invisible(c)) return false;
	const prev = chars[i - 1];
	return !(c === EMOJI_SELECTOR && prev !== undefined && !/[\p{L}\p{N}\s]/u.test(prev) && !invisible(prev));
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
	if ([...name].some((_, i, all) => hiddenAt(all, i))) return 'Names can’t contain control or invisible characters.';
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
			.filter((_, i, all) => !hiddenAt(all, i))
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
