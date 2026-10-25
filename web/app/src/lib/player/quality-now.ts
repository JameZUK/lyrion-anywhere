// What the quality badge and the signal-path diagram are drawn from
// (docs/plans/signal-path-redesign.md): the facts the page has about the playing track, put
// into the shared model's inputs (`QualityInputs`, see quality-path.ts). Pure: the live view
// (quality-now.svelte.ts) reads the app's state and hands it here.
import { quality } from '#lib/lms/quality';
import { codec as codecOf, type Decoded, type Device, type FileFormat, type Origin, type QualityInputs } from './quality-path';
import { servedOf, sentOf, streamOf } from './stream-quality';
import type { Format } from '#lib/offline/types';

/** The Opus and MP3 downloads' bit rates (docs/PROTOCOL.md, `GET /v1/download/track/{id}`). */
const DOWNLOAD_KBPS: Record<Exclude<Format, 'original'>, number> = { mp3: 320, opus: 160 };

export interface PathFacts {
	/** Who plays: this browser through the gateway, another player, or this browser's downloads. */
	player: 'local' | 'remote' | 'offline';
	/** The other player's name (remote). */
	playerName?: string;
	/** The current track as LMS reports it (`type`, `samplerate`, `samplesize`, `bitrate`). */
	track?: { type?: unknown; samplerate?: unknown; samplesize?: unknown; bitrate?: unknown } | null;
	/** The download playing offline: its format, its codec, and the library file's type when kept. */
	download?: { format: Format; codec: string; source?: string } | null;
	/** The worker's status fields (local): what the engine decodes, what LMS served, what the gateway sent. */
	source?: unknown;
	served?: unknown;
	sent?: unknown;
	/** The rate the engine outputs at, and the output device's own rate (Hz), when known. */
	engineRate?: number | null;
	deviceRate?: number | null;
	/** LMS's volume is applied digitally here (below full scale). */
	volumeScaled?: boolean;
	/** Why the level in force is in force (local). */
	origin?: Origin | null;
	/** The track can be sought, so a change of level can take effect now. */
	seekable?: boolean;
}

const rate = (v: unknown): number | null => (typeof v === 'number' && Number.isSafeInteger(v) && v >= 1000 && v <= 50_000_000 ? v : null);

/** The library file from LMS's status tags, as numbers. */
export function fileOf(track: PathFacts['track']): FileFormat | null {
	const q = quality(track);
	if (!q || typeof track?.type !== 'string') return null;
	return { kind: track.type, rate: q.rate, bits: q.bits, kbps: q.kbps };
}

/** The download's label as Now playing shows it ("FLAC", "Opus 160", "MP3 320"). */
export function downloadLabel(d: NonNullable<PathFacts['download']>): string {
	if (d.format !== 'original') return `${d.format === 'opus' ? 'Opus' : 'MP3'} ${DOWNLOAD_KBPS[d.format]}`;
	return quality({ type: d.codec })?.label ?? '';
}

function downloadDecoded(d: NonNullable<PathFacts['download']>): Decoded | null {
	if (d.format === 'opus') return { codec: 'Opus', rate: 48_000, lossless: false };
	if (d.format === 'mp3') return { codec: 'MP3', lossless: false };
	const c = codecOf(d.codec);
	return c ? { codec: c[0], lossless: c[1] } : null;
}

/** The facts as the shared model's inputs. */
export function qualityInputs(f: PathFacts): QualityInputs {
	if (f.player === 'remote') return { player: { kind: 'remote', name: f.playerName ?? '' }, file: fileOf(f.track) };
	if (f.player === 'offline') {
		const d = f.download;
		if (!d) return { player: { kind: 'offline', format: '' } };
		// The library file is known when the download kept its type (or is the original itself).
		const kind = d.format === 'original' ? d.codec : d.source;
		return {
			player: { kind: 'offline', format: downloadLabel(d) },
			file: kind ? { kind } : null,
			decoded: downloadDecoded(d),
		};
	}
	const s = streamOf(f.source);
	const engineRate = rate(f.engineRate);
	const deviceRate = rate(f.deviceRate);
	const resampled = engineRate !== null && deviceRate !== null && engineRate !== deviceRate;
	const device: Device = {
		name: null,
		rate: deviceRate ?? engineRate,
		// The browser never says what reaches the hardware: never claimed bit-perfect.
		bit_perfect: false,
		engine_bit_perfect: !resampled && f.volumeScaled !== true,
		resampled,
		dsp_active: false,
	};
	return {
		player: { kind: 'local' },
		file: fileOf(f.track),
		served: servedOf(f.served),
		sent: sentOf(f.sent),
		decoded: s ? { codec: s.codec, rate: engineRate, bits: s.bits, channels: s.channels ?? null, lossless: s.lossless } : null,
		device,
		origin: f.origin ?? null,
		seekable: f.seekable === true,
	};
}
