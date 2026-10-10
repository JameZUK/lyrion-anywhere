// A fake gateway for the download endpoints (GET /v1/download/track/{id}/info and
// GET /v1/download/track/{id}?format=), with Range support and hooks to misbehave.
import type { StoredTrack } from '$lib/offline/types';

export interface FakeTrack {
	id: number;
	key: string;
	title?: string;
	artist?: string;
	album?: string;
	album_id?: number;
	tracknum?: number;
	bytes: Uint8Array;
	formats?: string[];
}

export interface Req {
	url: string;
	path: string;
	format: string | null;
	auth: string;
	client: string;
	range: string | null;
}

export type Hook = (req: Req, n: number) => Response | Promise<Response> | undefined;

export const key = (n: number) => n.toString(16).padStart(64, '0');

export function bytes(n: number, seed = 1): Uint8Array {
	const b = new Uint8Array(n);
	for (let i = 0; i < n; i++) b[i] = (i * 31 + seed) & 0xff;
	return b;
}

/** A body that arrives in pieces, optionally breaking after `cutAfter` bytes. */
export function body(data: Uint8Array, piece = 256 * 1024, cutAfter?: number, gate?: Promise<void>): ReadableStream<Uint8Array> {
	let off = 0;
	return new ReadableStream<Uint8Array>({
		async pull(ctl) {
			if (gate) await gate;
			if (cutAfter !== undefined && off >= cutAfter) {
				ctl.error(new TypeError('network error'));
				return;
			}
			if (off >= data.length) {
				ctl.close();
				return;
			}
			const end = Math.min(data.length, off + piece, cutAfter ?? Infinity);
			ctl.enqueue(data.slice(off, end));
			off = end;
		},
	});
}

export function fakeServer(tracks: FakeTrack[]) {
	const reqs: Req[] = [];
	const hooks: Hook[] = [];
	let n = 0;
	const byId = new Map(tracks.map((t) => [t.id, t]));
	const fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
		const url = String(input);
		const h = new Headers(init?.headers);
		const u = new URL(url, 'http://gateway.example');
		const req: Req = { url, path: u.pathname, format: u.searchParams.get('format'), auth: h.get('authorization') ?? '', client: h.get('x-anywhere-client') ?? '', range: h.get('range') };
		reqs.push(req);
		const i = n++;
		if (init?.signal?.aborted) throw new DOMException('aborted', 'AbortError');
		for (const hook of hooks) {
			const r = await hook(req, i);
			if (r) return r;
		}
		const m = u.pathname.match(/^\/v1\/download\/track\/(\d+)(\/info)?$/);
		const t = m ? byId.get(Number(m[1])) : undefined;
		if (!t) return new Response('{"error":"not found"}', { status: 404 });
		if (m![2]) {
			return Response.json({
				id: t.id,
				key: t.key,
				title: t.title ?? `Track ${t.id}`,
				artist: t.artist ?? 'Artist',
				album: t.album ?? 'Album',
				album_id: t.album_id ?? 7,
				tracknum: t.tracknum ?? 1,
				duration: 241.2,
				original: { format: 'flc', content_type: 'audio/flac', size: t.bytes.length, ext: 'flac' },
				formats: t.formats ?? ['original', 'mp3', 'opus'],
			});
		}
		const rm = req.range?.match(/^bytes=(\d+)-$/);
		if (rm) {
			const start = Number(rm[1]);
			const part = t.bytes.slice(start);
			return new Response(body(part), {
				status: 206,
				headers: { 'content-length': String(part.length), 'content-range': `bytes ${start}-${t.bytes.length - 1}/${t.bytes.length}`, 'content-type': 'audio/flac' },
			});
		}
		return new Response(body(t.bytes), { status: 200, headers: { 'content-length': String(t.bytes.length), 'accept-ranges': 'bytes', 'content-type': 'audio/flac' } });
	}) as typeof globalThis.fetch;
	return { fetch, reqs, hooks };
}

export async function blobBytes(b: Blob): Promise<Uint8Array> {
	return new Uint8Array(await b.arrayBuffer());
}

/** Byte-for-byte equal (fast: toEqual on megabytes is very slow). */
export async function sameBytes(b: Blob | undefined, want: Uint8Array): Promise<boolean> {
	if (!b) return false;
	return Buffer.from(await blobBytes(b)).equals(Buffer.from(want));
}

export function stored(over: Partial<StoredTrack> = {}): StoredTrack {
	return {
		user: 'alice',
		key: key(1),
		id: 1,
		title: 'Song',
		artist: 'Artist',
		album: 'Album',
		albumId: 7,
		tracknum: 1,
		duration: 200,
		format: 'mp3',
		codec: 'mp3',
		ext: 'mp3',
		size: 4,
		contentType: 'audio/mpeg',
		downloadedAt: 1,
		chunks: 1,
		...over,
	};
}
