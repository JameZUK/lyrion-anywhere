// The two-step QR code arrives as SVG text from the gateway. It is shown as an image (a data
// URL), never inserted as markup, so it cannot run anything. `btoa` alone throws on any
// character outside Latin-1 (a non-ASCII account or server name in the SVG's title, say), so
// the text is encoded as UTF-8 first.

/** The most SVG text accepted (a QR code is a few kilobytes). */
export const MAX_QR_SVG = 256 * 1024;

/** A data URL showing `svg`, or '' when there is no usable SVG to show. */
export function svgDataUrl(svg: unknown): string {
	if (typeof svg !== 'string' || !svg.trim() || svg.length > MAX_QR_SVG) return '';
	try {
		const bytes = new TextEncoder().encode(svg);
		let binary = '';
		for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
		return `data:image/svg+xml;base64,${btoa(binary)}`;
	} catch {
		return '';
	}
}
