// Motion preferences beyond CSS's `prefers-reduced-motion`.

/**
 * Whether the ambient background should hold still on this device: few processor cores or
 * little memory (a low-power device), or the browser asking to save data. Browsers don't
 * expose battery saver; they slow animations down themselves when it is on.
 */
export function ambientStill(nav: { hardwareConcurrency?: number; deviceMemory?: number; connection?: { saveData?: boolean } } | undefined): boolean {
	if (!nav) return false;
	const cores = Number(nav.hardwareConcurrency);
	const memory = Number(nav.deviceMemory);
	if (Number.isFinite(cores) && cores > 0 && cores <= 2) return true;
	if (Number.isFinite(memory) && memory > 0 && memory <= 2) return true;
	return nav.connection?.saveData === true;
}

/** Whether the user asked for less motion (for transitions run from script). */
export function reducedMotion(): boolean {
	try {
		return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
	} catch {
		return false;
	}
}
