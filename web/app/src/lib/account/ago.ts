// How long ago, in words, for the device lists (Your devices, and after a password change).

/** "just now", "5 minutes ago", "3 hours ago", "2 days ago", "4 months ago". */
export function ago(secs: number): string {
	if (!(secs >= 120)) return 'just now'; // also a clock that is behind, or nonsense
	const m = Math.floor(secs / 60);
	if (m < 120) return `${m} minutes ago`;
	const h = Math.floor(m / 60);
	if (h < 48) return `${h} hours ago`;
	const days = Math.floor(h / 24);
	if (days < 60) return `${days} days ago`;
	return `${Math.floor(days / 30)} months ago`;
}
