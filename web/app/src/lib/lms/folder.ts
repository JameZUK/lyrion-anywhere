// Music folder entries, as the app shows them. LMS's answer is untrusted: ids must be real
// library ids, names are plain text, and nothing here ever holds a path (the gateway strips
// LMS's `url`; a name that still looks like a path is not shown either).
import { coverPath, id } from './commands';
import { textOf } from './format';
import type { FolderItem } from './types';

export type FolderRow =
	| { kind: 'folder'; id: number; name: string }
	| { kind: 'track'; id: number; name: string; cover: string | null; duration: string }
	| { kind: 'playlist'; id: number; name: string }
	| { kind: 'other'; id: number; name: string };

/** A name that is (or contains) a filesystem path: shown as its last part only. */
function baseName(s: string): string {
	const parts = s.split(/[\\/]/).filter(Boolean);
	return /^[a-z]:[\\/]|^[\\/]/i.test(s) ? (parts.at(-1) ?? '') : s;
}

export function folderRow(x: FolderItem | null | undefined): FolderRow | null {
	if (!x || typeof x !== 'object') return null;
	let n: number;
	try {
		n = id(x.id);
	} catch {
		return null;
	}
	const name = baseName(textOf(x.title) || textOf(x.filename)) || 'Untitled';
	switch (x.type) {
		case 'folder':
			return { kind: 'folder', id: n, name };
		case 'track':
			return { kind: 'track', id: n, name, cover: coverPath(x.coverid, 120), duration: /^\d{1,3}(:\d{2}){1,2}$/.test(x.duration ?? '') ? x.duration! : '' };
		case 'playlist':
			return { kind: 'playlist', id: n, name };
		default:
			return { kind: 'other', id: n, name };
	}
}

/** Folder names seen while browsing, so a folder's page can show its name. */
const names = new Map<number, string>();
export function rememberName(row: FolderRow) {
	if (row.kind !== 'folder') return;
	if (names.size > 2000) names.clear();
	names.set(row.id, row.name);
}
export const folderName = (folder: number): string | undefined => names.get(folder);
