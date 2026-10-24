#!/usr/bin/env node
// Opt-in end-to-end check of how the installed web app (the PWA) survives an update, in a real
// browser through Playwright with a persistent profile (so the service worker and its caches
// outlive each "launch"). Never run in CI: it needs a built gateway, a previous release's web
// build, this build, and Playwright with its browsers installed. It starts the gateway itself
// on loopback (no LMS needed: the sign-in page is enough to show the app started).
//
//   GATEWAY=target/debug/anywhere-gateway \
//     OLD_WEB=/path/to/previous-release/web NEW_WEB=web/app/build NEWER_WEB=/path/to/second-build \
//     PLAYWRIGHT_DIR=/path/to/node_modules/playwright \
//     node web/app/tests/e2e/pwa-update.mjs
//
// NEWER_WEB is a second `npm run build` of the same code (each build has its own version), for
// the "fault-new" scenario; without it that scenario is skipped. SCENARIOS picks some of:
//   auto       first launch on OLD_WEB, relaunch on NEW_WEB: the new version starts, and its
//              worker is applied by itself within seconds (nothing plays): the controller
//              changes, the page reloads once, only the new shell cache is left.
//   broken     first launch on NEW_WEB, relaunch on a copy whose start-up script is missing: the
//              boot guard clears the worker and the shell caches and reloads once; a second
//              broken load within five minutes does not reload again; a good server recovers.
//   fault-old  the white-screen bug in a previous release: its worker's Cache API writes fail
//              (injected), so a relaunch on NEW_WEB gets the old stored page, whose start-up
//              script the server no longer has. Reported, not asserted (it is the old code).
//   fault-new  the same fault with this code (NEW_WEB, then NEWER_WEB): the fresh page starts.
// Exit status 1 if an assertion fails. PORT (default 19781), BROWSER (default chromium).
import { spawn } from 'node:child_process';
import { cpSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const env = process.env;
const GATEWAY = env.GATEWAY ?? '';
const OLD_WEB = env.OLD_WEB ?? '';
const NEW_WEB = resolve(env.NEW_WEB ?? 'web/app/build');
const NEWER_WEB = env.NEWER_WEB ?? '';
const PORT = Number(env.PORT ?? 19781);
const URL_ = `http://127.0.0.1:${PORT}/`;
const SCENARIOS = (env.SCENARIOS ?? 'auto,broken,fault-old,fault-new').split(',').map((s) => s.trim());
if (!GATEWAY || !OLD_WEB || !env.PLAYWRIGHT_DIR) {
	console.error('Set GATEWAY, OLD_WEB, NEW_WEB, PLAYWRIGHT_DIR (see the comment at the top of this file).');
	process.exit(2);
}
const pw = (await import(join(resolve(env.PLAYWRIGHT_DIR), 'index.mjs'))).default;
const engine = pw[env.BROWSER ?? 'chromium'];
const work = mkdtempSync(join(tmpdir(), 'pwa-update-'));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (ok, what) => {
	console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${what}`);
	if (!ok) failures++;
};

const bootOf = (dir) => readFileSync(join(dir, 'index.html'), 'utf8').match(/_app\/boot\.[0-9a-f]+\.js/)?.[0] ?? null;
const versionOf = (dir) => JSON.parse(readFileSync(join(dir, '_app', 'version.json'), 'utf8')).version;

let variants = 0;
/**
 * A copy of a build, changed by `edit(dir)`. Its files get their own modification time, a
 * minute after the previous copy's: the gateway answers If-Modified-Since by the second, so
 * two copies made within one second would otherwise answer 304 for each other's index.html.
 */
function variant(from, name, edit) {
	const dir = join(work, name);
	cpSync(from, dir, { recursive: true });
	edit(dir);
	const t = Date.now() / 1000 + 60 * ++variants;
	for (const f of readdirSync(dir, { recursive: true })) utimesSync(join(dir, String(f)), t, t);
	return dir;
}
/** A copy whose index.html references a start-up script that does not exist. */
const brokenCopy = (from) => variant(from, 'broken', (d) => renameSync(join(d, bootOf(from)), join(d, `${bootOf(from)}.gone`)));
/**
 * A copy whose worker's Cache API writes fail once the page has created the cache
 * "e2e-fault-on" (as a home-screen app's storage can fail): put/add/addAll reject.
 */
const FAULT = `// e2e fault injection (tests/e2e/pwa-update.mjs)
(() => {
	const open = caches.open.bind(caches), has = caches.has.bind(caches);
	caches.open = async (name) => {
		const c = await open(name);
		if (name === 'e2e-fault-on') return c;
		for (const m of ['put', 'add', 'addAll']) {
			const real = c[m].bind(c);
			c[m] = async (...a) => ((await has('e2e-fault-on')) ? Promise.reject(new DOMException('injected', 'UnknownError')) : real(...a));
		}
		return c;
	};
})();
`;
const faultyCopy = (from, name) => variant(from, name, (d) => writeFileSync(join(d, 'service-worker.js'), FAULT + readFileSync(join(d, 'service-worker.js'), 'utf8')));

let gw = null;
async function serve(dir) {
	await stop();
	const data = mkdtempSync(join(work, 'data-'));
	gw = spawn(GATEWAY, ['serve', '--data-dir', data, '--listen', `127.0.0.1:${PORT}`, '--admin-listen', `127.0.0.1:${PORT + 2}`, '--no-tunnel', '--no-transcode', '--no-analysis', '--static-dir', dir], { stdio: 'ignore' });
	for (let i = 0; i < 100; i++) {
		try {
			if ((await fetch(`${URL_}healthz`)).ok) return;
		} catch {
			/* not yet */
		}
		await wait(200);
	}
	throw new Error('the gateway did not start');
}
async function stop() {
	if (!gw) return;
	const p = gw;
	gw = null;
	p.kill();
	await new Promise((r) => (p.exitCode !== null ? r() : p.once('exit', r)));
}

/** One launch of the installed app: a persistent profile, one page, its reloads counted. */
async function launch(profile) {
	const ctx = await engine.launchPersistentContext(profile, { headless: env.HEADED !== '1' });
	const page = await ctx.newPage();
	// navigations: page loads (a reload counts; the hash router's own navigations do not).
	const log = { navigations: 0, missing: [] };
	page.on('load', () => log.navigations++);
	page.on('response', (r) => r.status() === 404 && log.missing.push(new URL(r.url()).pathname));
	await page.goto(URL_, { waitUntil: 'load', timeout: 30000 });
	return { ctx, page, log };
}
const started = (page, ms = 15000) =>
	page
		.waitForSelector('input[type=password]', { timeout: ms })
		.then(() => true)
		.catch(() => false);
const state = (page) =>
	page.evaluate(async () => {
		const regs = navigator.serviceWorker ? await navigator.serviceWorker.getRegistrations() : [];
		const reg = regs[0];
		return {
			booted: document.documentElement.dataset.laBooted === '1',
			boot: document.querySelector('script[src*="_app/boot."]')?.getAttribute('src')?.replace(/^\.\//, '') ?? null,
			controller: navigator.serviceWorker?.controller?.scriptURL ?? null,
			registrations: regs.length,
			active: reg?.active?.state ?? null,
			waiting: reg?.waiting?.state ?? null,
			caches: (await caches.keys()).filter((k) => k.startsWith('anywhere-shell-')),
		};
	});
/** First launch: the worker installed, active and in control. */
async function firstLaunch(profile, fault = false) {
	const { ctx, page } = await launch(profile);
	await page.evaluate(async () => {
		await navigator.serviceWorker.ready;
		for (let i = 0; i < 100 && !navigator.serviceWorker.controller; i++) await new Promise((r) => setTimeout(r, 100));
	});
	if (fault) await page.evaluate(() => caches.open('e2e-fault-on'));
	const s = await state(page);
	await ctx.close();
	return s;
}

async function auto() {
	console.log(`auto: first launch on ${versionOf(OLD_WEB)}, relaunch on ${versionOf(NEW_WEB)}`);
	const profile = join(work, 'profile-auto');
	await serve(OLD_WEB);
	const first = await firstLaunch(profile);
	check(!!first.controller && first.caches.length === 1, `first launch controlled, one shell cache (${first.caches})`);
	await serve(NEW_WEB);
	const t0 = Date.now();
	const { ctx, page, log } = await launch(profile);
	check(await started(page), 'the relaunch starts');
	const s = await state(page);
	check(s.boot === bootOf(NEW_WEB) && s.booted, `it runs the new version (${s.boot})`);
	let after = s;
	while (Date.now() - t0 < 20000) {
		await wait(500);
		try {
			after = await state(page);
		} catch {
			continue; // reloading
		}
		if (log.navigations >= 2 && after.waiting === null && after.caches.length === 1 && after.caches[0] === `anywhere-shell-${versionOf(NEW_WEB)}`) break;
	}
	const ms = Date.now() - t0;
	check(log.navigations === 2, `the page reloaded once by itself (${log.navigations - 1} reload(s), ${ms} ms after launch)`);
	check(after.waiting === null && after.active === 'activated' && !!after.controller, 'the new worker controls the page, nothing waiting');
	check(after.caches.length === 1 && after.caches[0] === `anywhere-shell-${versionOf(NEW_WEB)}`, `only the new shell cache is left (${after.caches})`);
	check(await started(page), 'and the app is running after the reload');
	await ctx.close();
}

async function broken() {
	const bad = brokenCopy(NEW_WEB);
	console.log(`broken: first launch on ${versionOf(NEW_WEB)}, relaunch on a copy without ${bootOf(NEW_WEB)}`);
	const profile = join(work, 'profile-broken');
	await serve(NEW_WEB);
	await firstLaunch(profile);
	await serve(bad);
	const t0 = Date.now();
	const { ctx, page, log } = await launch(profile);
	check(!(await started(page, 5000)), 'the broken page does not start');
	check(log.missing.includes(`/${bootOf(NEW_WEB)}`), 'its start-up script is missing (404)');
	while (log.navigations < 2 && Date.now() - t0 < 25000) await wait(250);
	await page.waitForLoadState('load');
	check(log.navigations === 2, `the guard reloaded once (${Date.now() - t0} ms after launch)`);
	const s = await state(page);
	check(s.registrations === 0 && s.caches.length === 0, `worker unregistered (${s.registrations}), shell caches gone (${s.caches})`);
	const stamp = await page.evaluate(() => localStorage.getItem('la-boot-guard'));
	check(!!stamp, 'the attempt is noted');
	await wait(15000);
	check(log.navigations === 2, 'the second broken load (within five minutes) does not reload again');
	await ctx.close();
	// A relaunch, still broken and within five minutes: no loop either.
	const again = await launch(profile);
	await wait(13000);
	check(again.log.navigations === 1, 'a broken relaunch within five minutes is left alone');
	await again.ctx.close();
	await serve(variant(NEW_WEB, 'fixed', () => {})); // a deployment newer than the broken one
	const good = await launch(profile);
	check(await started(good.page), 'once the server is fixed, the app starts');
	await good.ctx.close();
}

async function faultOld() {
	const oldF = faultyCopy(OLD_WEB, 'old-faulty');
	const newF = faultyCopy(NEW_WEB, 'new-faulty');
	console.log(`fault-old: ${versionOf(OLD_WEB)} with failing cache writes, relaunch on ${versionOf(NEW_WEB)} (report only)`);
	const profile = join(work, 'profile-fault-old');
	await serve(oldF);
	await firstLaunch(profile, true);
	await serve(newF);
	for (const n of [1, 2, 3]) {
		const { ctx, page, log } = await launch(profile);
		const ok = await started(page, 12000);
		const s = await state(page).catch(() => null);
		console.log(`  launch ${n}: started=${ok} boot=${s?.boot} missing=${JSON.stringify(log.missing)} active=${s?.active} waiting=${s?.waiting} controller=${s?.controller?.replace(URL_, '/')}`);
		await ctx.close();
	}
}

async function faultNew() {
	if (!NEWER_WEB) {
		console.log('fault-new: skipped (set NEWER_WEB to a second build of this code)');
		return;
	}
	const a = faultyCopy(NEW_WEB, 'a-faulty');
	const b = faultyCopy(NEWER_WEB, 'b-faulty');
	console.log(`fault-new: ${versionOf(NEW_WEB)} with failing cache writes, relaunch on ${versionOf(NEWER_WEB)}`);
	const profile = join(work, 'profile-fault-new');
	await serve(a);
	const first = await firstLaunch(profile, true);
	check(first.caches.length === 1, 'the first version stored its shell before the fault');
	await serve(b);
	const t0 = Date.now();
	const { ctx, page, log } = await launch(profile);
	check(await started(page), 'the relaunch starts, although the cache writes fail');
	const s = await state(page);
	check(s.boot === bootOf(NEWER_WEB), `with the fresh page (${s.boot}), not the stored one (${bootOf(NEW_WEB)})`);
	check(!log.missing.length, `nothing missing (${log.missing})`);
	while (log.navigations < 2 && Date.now() - t0 < 20000) await wait(250);
	await page.waitForLoadState('load');
	const after = await state(page).catch(() => null);
	check(log.navigations === 2 && after?.waiting === null && after?.active === 'activated', `the new worker installed anyway and applied itself (${log.navigations - 1} reload(s))`);
	check(await started(page), 'the app is running after the reload');
	await ctx.close();
}

try {
	const all = { auto, broken, 'fault-old': faultOld, 'fault-new': faultNew };
	for (const name of SCENARIOS) await all[name]();
} finally {
	await stop();
	if (env.KEEP !== '1') rmSync(work, { recursive: true, force: true });
}
console.log(failures ? `${failures} check(s) failed` : 'all checks passed');
process.exit(failures ? 1 : 0);
