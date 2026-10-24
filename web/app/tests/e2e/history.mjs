#!/usr/bin/env node
// Opt-in end-to-end check of history in the web app (SvelteKit's hash router), in a real
// browser through Playwright. Never run in CI: it needs a built gateway, a built web app,
// python3 with Pillow (for tools/demo/fake_lms.py, the invented library it signs in to) and
// Playwright with its browsers installed. It starts the demo LMS and the gateway itself, on
// loopback, with a throwaway data directory and user.
//
//   GATEWAY=target/debug/anywhere-gateway WEB=web/app/build \
//     PLAYWRIGHT_DIR=/path/to/node_modules/playwright node web/app/tests/e2e/history.mjs
//
// It signs in, follows two in-app links, and checks that the browser's Back and Forward show
// the right page at the right address without a reload; that a route assigned to
// location.hash (an edited address bar, a link from elsewhere) opens in the app without a
// reload, and that Back from it returns to the page before (the browser's stray entry was
// stepped over); and that two routes assigned in the same instant leave the address and the
// page agreeing, without a reload (which of the two wins depends on the browser's event order,
// so that is reported, not asserted). Exit status 1 if a check fails.
//
// Why it exists: SvelteKit 3 moved its history bookkeeping to another key, and from 0.6.4 to
// 0.7.1 the app took every Back and Forward for an outside change (lib/nav.ts). jsdom cannot
// run the router, so only a browser shows this.
//
// PORT (default 19791) is the gateway's; its admin port is PORT+2 and the demo LMS PORT+4.
// BROWSER (default chromium). HEADED=1 shows the browser.
import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const env = process.env;
const GATEWAY = env.GATEWAY ?? '';
const WEB = resolve(env.WEB ?? 'web/app/build');
const PORT = Number(env.PORT ?? 19791);
const LMS_PORT = PORT + 4;
const BASE = `http://127.0.0.1:${PORT}/`;
const HEADED = env.HEADED === '1';
if (!GATEWAY || !env.PLAYWRIGHT_DIR) {
	console.error('Set GATEWAY, WEB and PLAYWRIGHT_DIR (see the comment at the top of this file).');
	process.exit(2);
}
const pw = (await import(join(resolve(env.PLAYWRIGHT_DIR), 'index.mjs'))).default;
const engine = pw[env.BROWSER ?? 'chromium'];
const FAKE_LMS = fileURLToPath(new URL('../../../../tools/demo/fake_lms.py', import.meta.url));
const USER = 'history-check';
const PASSWORD = randomBytes(12).toString('base64url');
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
const check = (ok, what) => {
	console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${what}`);
	if (!ok) failures++;
};
const note = (what) => console.log(`  note ${what}`);

// --- The demo LMS and the gateway ---------------------------------------------------------
const data = mkdtempSync(join(tmpdir(), 'history-check-'));
const children = [];
/** Wait for `child` to answer at `url` (any answer, or a 2xx when `ok`). */
async function up(child, url, what, ok = true) {
	for (let i = 0; i < 100; i++) {
		if (child.exitCode !== null) throw new Error(`${what} exited with ${child.exitCode}`);
		try {
			const r = await fetch(url);
			if (r.ok || !ok) return;
		} catch {
			/* not yet */
		}
		await wait(200);
	}
	throw new Error(`${what} did not start`);
}
async function start() {
	const lms = spawn('python3', [FAKE_LMS, '--port', String(LMS_PORT)], { stdio: ['ignore', 'ignore', 'inherit'] });
	children.push(lms);
	await up(lms, `http://127.0.0.1:${LMS_PORT}/`, 'the demo LMS (python3 with Pillow)', false);
	const made = spawnSync(GATEWAY, ['admin', '--data-dir', data, 'create-user', USER, '--role', 'listener'], { input: `${PASSWORD}\n`, stdio: ['pipe', 'ignore', 'inherit'] });
	if (made.status !== 0) throw new Error('could not create the user');
	const gw = spawn(
		GATEWAY,
		['serve', '--data-dir', data, '--listen', `127.0.0.1:${PORT}`, '--admin-listen', `127.0.0.1:${PORT + 2}`, '--lms', '127.0.0.1', '--lms-http-port', String(LMS_PORT), '--lms-cli-port', String(LMS_PORT + 1), '--lms-slimproto-port', String(LMS_PORT + 2), '--no-tunnel', '--no-transcode', '--no-analysis', '--static-dir', WEB],
		{ stdio: ['ignore', 'ignore', 'inherit'] },
	);
	children.push(gw);
	await up(gw, `${BASE}healthz`, 'the gateway');
}
async function stop() {
	for (const p of children.splice(0)) {
		p.kill();
		await new Promise((r) => (p.exitCode !== null ? r() : p.once('exit', r)));
	}
	rmSync(data, { recursive: true, force: true });
}

// --- The check ----------------------------------------------------------------------------
async function run() {
	const browser = await engine.launch({ headless: !HEADED });
	const ctx = await browser.newContext({ serviceWorkers: 'block', locale: 'en-GB' });
	const page = await ctx.newPage();
	const errors = [];
	page.on('pageerror', (e) => errors.push(`pageerror: ${e}`));
	page.on('console', (m) => {
		if (m.type() === 'error') errors.push(m.text());
	});
	const hash = () => page.evaluate(() => location.hash);
	const title = () => page.title();
	/** True while the page has not reloaded since the marker was set. */
	const alive = () => page.evaluate(() => globalThis.__historyCheck === 'alive');
	/** Follow an in-app link: the router's own navigation. */
	async function follow(h) {
		await page.evaluate((h) => {
			const a = document.createElement('a');
			a.href = h;
			a.textContent = 'e2e';
			document.body.append(a);
			a.click();
			a.remove();
		}, h);
		await page.waitForFunction((h) => location.hash === h, h, { timeout: 10_000 });
		await wait(500);
	}
	/** The address and the page shown agree, and nothing reloaded. */
	async function at(h, name, when) {
		const [now, t] = [await hash(), await title()];
		check(now === h, `${when}: the address is ${h} (it is ${now || '(none)'})`);
		check(new RegExp(name, 'i').test(t), `${when}: the page shown is ${name} (title: ${t})`);
		check(await alive(), `${when}: no reload`);
	}

	await page.goto(BASE);
	await page.fill('input[autocomplete=username]', USER);
	await page.fill('input[autocomplete=current-password]', PASSWORD);
	await page.click('button[type=submit]');
	await page.waitForSelector('main.content', { timeout: 20_000 });
	await wait(1000);
	await page.evaluate(() => {
		globalThis.__historyCheck = 'alive'; // a reload wipes it
	});
	console.log(`signed in (${await title()})`);

	console.log('two in-app links: #/settings, then #/search');
	await follow('#/settings');
	await follow('#/search');
	check(await alive(), 'following links: no reload');

	await page.goBack();
	await wait(1500);
	await at('#/settings', 'Settings', 'Back');

	await page.goForward();
	await wait(1500);
	await at('#/search', 'Search', 'Forward');

	console.log('a route assigned to location.hash: #/rooms');
	await page.evaluate(() => {
		location.hash = '#/rooms';
	});
	await wait(2500);
	await at('#/rooms', 'Rooms', 'the typed route');

	await page.goBack();
	await wait(1500);
	await at('#/search', 'Search', 'Back from the typed route');

	console.log('two routes assigned in the same instant: #/queue, #/alarms');
	await page.evaluate(() => {
		location.hash = '#/queue';
		location.hash = '#/alarms';
	});
	await wait(3000);
	const [h2, t2] = [await hash(), await title()];
	const shown = h2 === '#/queue' ? 'Queue' : h2 === '#/alarms' ? 'Alarms' : null;
	check(shown !== null && new RegExp(shown, 'i').test(t2), `one of the two is shown, and the address agrees (${h2 || '(none)'}: ${t2})`);
	check(await alive(), 'two at once: no reload');
	await page.goBack();
	await wait(1500);
	note(`Back from there lands on ${(await hash()) || '(none)'} (${await title()})`);
	check(await alive(), 'and that Back did not reload either');

	const unexpected = errors.filter((e) => !/status of 401/.test(e));
	check(unexpected.length === 0, `no console or page errors${unexpected.length ? `: ${unexpected.join(' | ')}` : ''}`);
	await browser.close();
}

try {
	await start();
	await run();
} catch (e) {
	console.error(e);
	failures++;
} finally {
	await stop();
}
console.log(failures ? `${failures} FAILED` : 'all passed');
process.exit(failures ? 1 : 0);
