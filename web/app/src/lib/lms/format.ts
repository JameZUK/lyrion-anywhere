// Display helpers. Everything returns plain text; the UI never renders LMS data as HTML.

/** 3:07, or 1:02:03 for an hour or more. */
export function duration(seconds: unknown): string {
	const s = typeof seconds === 'number' && Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	const ss = String(s % 60).padStart(2, '0');
	return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/** Any LMS value as display text: strings and numbers only, trimmed and length-capped. */
export function textOf(v: unknown, max = 300): string {
	if (typeof v === 'number' && Number.isFinite(v)) return String(v);
	if (typeof v !== 'string') return '';
	return v.length > max ? `${v.slice(0, max - 1)}…` : v;
}

/** "FLAC 44.1 kHz 16-bit", from whatever format details LMS reported. */
export function formatDetails(t: { type?: string; samplerate?: string; samplesize?: string; bitrate?: string }): string {
	const parts: string[] = [];
	if (t.type) parts.push(textOf(t.type, 20).toUpperCase());
	const rate = Number(t.samplerate);
	if (rate > 0) parts.push(`${(rate / 1000).toLocaleString('en-GB', { maximumFractionDigits: 1 })} kHz`);
	const size = Number(t.samplesize);
	if (size > 0) parts.push(`${size}-bit`);
	else if (t.bitrate) parts.push(textOf(t.bitrate, 20));
	return parts.join(' ');
}
