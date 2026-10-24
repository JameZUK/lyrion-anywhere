// Boot guard. If the app has not started 10 s after the page loaded (a stale or broken stored
// app shell), and the server answers, it removes this app's service worker and stored shell
// and reloads: at most once every five minutes (a broken server shows its broken page rather
// than a reload loop), never offline. A classic script (no modules, nothing inline: the CSP is
// script-src 'self'). The app marks itself started with data-la-booted on <html>.
(function (w) {
	'use strict';
	var KEY = 'la-boot-guard';
	var WAIT_MS = 10000;
	var GAP_MS = 5 * 60 * 1000;
	var PREFIX = 'anywhere-shell-';

	/** 'repair' or 'none', from { booted, online, now, last } (last: the previous repair's time, or null). */
	function decide(s) {
		if (s.booted || !s.online) return 'none';
		var since = typeof s.last === 'number' ? s.now - s.last : Infinity;
		return since >= 0 && since < GAP_MS ? 'none' : 'repair';
	}
	function last() {
		try {
			var v = Number(w.localStorage.getItem(KEY));
			return isFinite(v) && v > 0 ? v : null;
		} catch (e) {
			return null;
		}
	}
	/** The server answers (a query keeps the service worker out of it). */
	function reachable() {
		return w.fetch('./?la-boot-check=' + Date.now(), { cache: 'no-store', credentials: 'omit' }).then(function (r) { return !!r && r.ok; }, function () { return false; });
	}
	// Clears, then reloads only if the attempt could be noted (no record, no loop guard: the
	// next launch then starts clean instead).
	function repair() {
		var noted = true;
		try {
			w.localStorage.setItem(KEY, String(Date.now()));
		} catch (e) {
			noted = false;
		}
		var quiet = function () {};
		var sw = w.navigator.serviceWorker;
		var regs = sw && sw.getRegistrations ? sw.getRegistrations().then(function (rs) { return Promise.all(rs.map(function (r) { return r.unregister(); })); }) : null;
		var stores = w.caches ? w.caches.keys().then(function (ks) { return Promise.all(ks.filter(function (k) { return k.indexOf(PREFIX) === 0; }).map(function (k) { return w.caches.delete(k); })); }) : null;
		return Promise.all([regs && regs.catch(quiet), stores && stores.catch(quiet)]).then(function () { if (noted) w.location.reload(); return noted; });
	}
	/** Look once: repairs (and reloads) when it must; resolves to whether it reloaded. */
	function check() {
		var booted = w.document.documentElement.getAttribute('data-la-booted') === '1';
		if (decide({ booted: booted, online: w.navigator.onLine !== false, now: Date.now(), last: last() }) !== 'repair') return Promise.resolve(false);
		return reachable().then(function (ok) { return ok && !w.document.documentElement.getAttribute('data-la-booted') ? repair() : false; });
	}
	function arm() {
		var start = function () { w.setTimeout(check, WAIT_MS); };
		if (w.document.readyState === 'complete') start();
		else w.addEventListener('load', start, { once: true });
	}
	w.laBootGuard = { decide: decide, check: check, arm: arm, KEY: KEY, WAIT_MS: WAIT_MS, GAP_MS: GAP_MS, PREFIX: PREFIX };
	arm();
})(window);
