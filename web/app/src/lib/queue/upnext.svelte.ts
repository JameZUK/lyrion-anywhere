// The app's one Up next tracker (see tracker.ts), with a version number views can watch.
import { UpNextTracker } from './tracker';

class Version {
	n = $state(0);
}
const version = new Version();

export const upNextTracker = new UpNextTracker();
upNextTracker.changed = () => {
	version.n++;
};

/** Read in a view to re-render when the tracked sections change. */
export function trackerVersion(): number {
	return version.n;
}
