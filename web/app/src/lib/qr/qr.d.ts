// Types for qr.js (plain JavaScript, shared with the LMS plugin's settings page).

export interface QrSymbol {
	version: number;
	/** Modules per side. */
	size: number;
	mask: number;
	/** Row-major, 1 for dark. */
	modules: Uint8Array;
}

export const MAX_VERSION: number;
export const PAIRING_SCHEME: string;
export function capacity(version: number): number;
export function utf8(text: string): number[];
export function reedSolomon(data: number[], n: number): number[];
export function formatBits(mask: number): number;
export function versionBits(version: number): number;
export function penalty(size: number, dark: Uint8Array): number;
export function encodeQr(text: string, options?: { mask?: number }): QrSymbol;
export function qrPath(qr: QrSymbol, quiet?: number): string;
export function normalisePairingCode(code: unknown): string | null;
export function pairingServer(address: unknown): string | null;
export function pairingLink(server: unknown, code: unknown): string | null;
export function groupPairingCode(code: unknown): string;
