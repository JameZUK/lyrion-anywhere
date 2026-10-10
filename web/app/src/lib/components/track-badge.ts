/** A small status mark shown on a track row (TrackList's `badge`). */
export interface TrackBadge {
	kind: 'done' | 'busy' | 'failed';
	/** Read out by screen readers, and shown as a tooltip. */
	label: string;
}
