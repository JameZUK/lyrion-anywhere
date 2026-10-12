export function decodeQr(
	size: number,
	get: (x: number, y: number) => number,
): { text: string; bytes: number[]; version: number; mask: number };
