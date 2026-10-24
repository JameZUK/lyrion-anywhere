#!/usr/bin/env node
// Opt-in end-to-end check of the web app over plain http:// to a private address (a trusted
// network, SPEC D28), in real browsers through Playwright. Never run in CI: it needs a running
// gateway bound to a private (LAN or VPN) address with --trusted-network, an LMS behind it
// with at least one album, and Playwright with its browsers installed.
//
//   anywhere-gateway serve --data-dir <dir> --listen 10.0.0.10:9443 \
//     --trusted-network 10.0.0.10/32 --static-dir web/app/build --no-tunnel --lms 127.0.0.1
//   ADDRESS=10.0.0.10 PORT=9443 USERNAME=alice PASSWORD=... \
//     BROWSERS=chromium,firefox PLAYWRIGHT_DIRS=/path/to/node_modules/playwright \
//     node web/app/tests/e2e/insecure-context.mjs
//
// PLAYWRIGHT_DIRS is one Playwright package directory for every browser, or a comma-separated
// list of browser=directory pairs (chromium=/a/node_modules/playwright,firefox=/b/...). Use a
// throwaway LMS (or a dedicated, muted test player's server): the check plays a track on the
// browser's own player for about ten seconds, and downloads an album into the browser.
// Per browser it checks: the page is an insecure context; the unencrypted-connection notice
// shows and can be dismissed for the session; sign-in works and survives a reload (the
// refresh cookie without Secure); the playback mode is the media element; a track plays
// with the position advancing; the downloads pages work; and no unexpected console errors
// or page errors appear. Exit status 1 if anything fails. HEADED=1 shows the browsers.
import { pathToFileURL } from 'node:url';

const env = process.env;
const ADDRESS = env.ADDRESS ?? '';
const PORT = env.PORT ?? '9443';
const USERNAME = env.USERNAME ?? '';
const PASSWORD = env.PASSWORD ?? '';
const BROWSERS = (env.BROWSERS ?? 'chromium,firefox')
	.split(',')
	.map((b) => b.trim())
	.filter(Boolean);
const PLAY_SECONDS = Number(env.PLAY_SECONDS ?? 10);
const HEADED = env.HEADED === '1';

if (!ADDRESS || !USERNAME || !PASSWORD || !env.PLAYWRIGHT_DIRS) {
	console.error('Set ADDRESS, PORT, USERNAME, PASSWORD and PLAYWRIGHT_DIRS (see the comment at the top of this file).');
	process.exit(2);
}
const BASE = `http://${ADDRESS.includes(':') ? `[${ADDRESS}]` : ADDRESS}:${PORT}/`;

function playwrightDir(browser) {
	const spec = env.PLAYWRIGHT_DIRS;
	if (!spec.includes('=')) return spec;
	for (const pair of spec.split(',')) {
		const [name, dir] = pair.split('=');
		if (name?.trim() === browser && dir) return dir.trim();
	}
	throw new Error(`no Playwright directory for ${browser} in PLAYWRIGHT_DIRS`);
}

/** Console errors that are expected over plain http:// (each with its reason). */
const EXPECTED_ERRORS = [
	// Chromium ignores the gateway's Cross-Origin-Opener-Policy on an untrustworthy origin,
	// and says so as an error. Harmless: the header is defence in depth.
	/Cross-Origin-Opener-Policy header has been ignored/,
];
/** Before signing in, the silent sign-in (no refresh cookie yet) is refused: expected. */
const EXPECTED_BEFORE_SIGN_IN = [/status of 401/];

/**
 * Move within the app (a hash route) by clicking a link to it, so SvelteKit's router takes
 * it and this device's player keeps running (assigning location.hash would not do).
 */
async function go(page, hash) {
	const before = await page.evaluate(() => location.hash);
	await page.evaluate((h) => {
		const a = document.createElement('a');
		a.href = h;
		a.textContent = 'e2e';
		document.body.append(a);
		a.click();
		a.remove();
	}, hash);
	// Some routes move on (#/albums opens Library), so wait for any route but the old one.
	if (before !== hash) await page.waitForFunction((b) => location.hash !== b, before, { timeout: 10_000 });
	await page.waitForTimeout(500);
}

/** The position of the playing media element(s), in seconds (0 when none plays). */
const position = () => Math.max(0, ...[...document.querySelectorAll('audio')].map((a) => (a.paused ? 0 : a.currentTime)));

async function check(browserName) {
	const pw = await import(pathToFileURL(`${playwrightDir(browserName)}/index.mjs`).href);
	const type = pw[browserName];
	if (!type) throw new Error(`Playwright has no ${browserName}`);
	const browser = await type.launch({
		headless: !HEADED,
		// Let the media element start without a real person's gesture (headless has none).
		...(browserName === 'chromium' ? { args: ['--autoplay-policy=no-user-gesture-required'] } : {}),
		...(browserName === 'firefox' ? { firefoxUserPrefs: { 'media.autoplay.default': 0, 'media.autoplay.blocking_policy': 0 } } : {}),
	});
	const results = [];
	const errors = [];
	const ok = (name, pass, detail = '') => {
		results.push({ name, pass: !!pass, detail });
		console.log(`  ${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `: ${detail}` : ''}`);
	};
	try {
		const context = await browser.newContext();
		const page = await context.newPage();
		let signedIn = false;
		page.on('console', (m) => {
			if (m.type() !== 'error') return;
			const expected = [...EXPECTED_ERRORS, ...(signedIn ? [] : EXPECTED_BEFORE_SIGN_IN)];
			if (!expected.some((r) => r.test(m.text()))) errors.push(`console: ${m.text()}`);
		});
		page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));

		await page.goto(BASE);
		const apis = await page.evaluate(() => ({
			isSecureContext: globalThis.isSecureContext,
			locks: 'locks' in navigator,
			storage: 'storage' in navigator,
			serviceWorker: 'serviceWorker' in navigator,
			caches: 'caches' in globalThis,
			randomUUID: typeof crypto.randomUUID === 'function',
			subtle: !!crypto.subtle,
			wakeLock: 'wakeLock' in navigator,
			AudioWorkletNode: typeof AudioWorkletNode !== 'undefined',
			audioWorklet: typeof AudioContext !== 'undefined' && 'audioWorklet' in AudioContext.prototype,
			indexedDB: typeof indexedDB !== 'undefined',
			MediaSource: typeof MediaSource !== 'undefined',
			flacInMp4: typeof MediaSource !== 'undefined' && MediaSource.isTypeSupported('audio/mp4; codecs="flac"'),
		}));
		console.log(`  page APIs: ${JSON.stringify(apis)}`);
		ok('the page is an insecure context', apis.isSecureContext === false);

		// The notice, before signing in.
		const notice = page.locator('[data-insecure-notice]');
		await notice.waitFor({ timeout: 15_000 }).catch(() => {});
		ok('the unencrypted-connection notice shows on the sign-in page', (await notice.count()) === 1, ((await notice.textContent().catch(() => '')) ?? '').trim());

		// Sign in.
		await page.getByLabel('Username').fill(USERNAME);
		await page.getByLabel('Password').fill(PASSWORD);
		await page.getByRole('button', { name: 'Sign in' }).click();
		const shell = page.locator('nav.side');
		await shell.waitFor({ timeout: 20_000 }).catch(() => {});
		signedIn = true;
		ok('sign-in succeeds', (await shell.count()) === 1);
		const refresh = (await context.cookies()).find((c) => c.path?.startsWith('/v1/auth'));
		ok(
			'the refresh cookie is set without Secure, and HttpOnly',
			!!refresh && refresh.secure === false && refresh.httpOnly === true,
			refresh ? `secure=${refresh.secure} httpOnly=${refresh.httpOnly} sameSite=${refresh.sameSite}` : 'no cookie',
		);

		// Dismiss the notice; it stays dismissed after a reload, and the sign-in survives it.
		ok('the notice shows in the app too', (await notice.count()) === 1);
		await page.locator('[data-insecure-dismiss]').first().click();
		ok('Dismiss hides the notice', (await notice.count()) === 0);
		await page.reload();
		await shell.waitFor({ timeout: 20_000 }).catch(() => {});
		ok('the sign-in survives a reload', (await shell.count()) === 1 && (await page.getByRole('button', { name: 'Sign in' }).count()) === 0);
		ok('the notice stays dismissed for the session', (await notice.count()) === 0);

		// The playback engine chosen automatically, and what the diagnostics report.
		await go(page, '#/settings/advanced');
		await page.getByText('Playback engine').first().waitFor({ timeout: 10_000 }).catch(() => {});
		const planned = ((await page.locator('main').textContent()) ?? '').match(/Playback engine\s*(Media element|Web Audio):/)?.[1] ?? '?';
		ok('the planned playback engine is the media element', planned === 'Media element', planned);
		const worklet = ((await page.locator('details[data-developer]').textContent()) ?? '').match(/AudioWorklet\s*(yes|no)/)?.[1];
		ok('the diagnostics report no AudioWorklet', worklet === 'no', `AudioWorklet: ${worklet}`);
		// This device's player starts by itself after signing in (SPEC D20); wait for it.
		const ready = await page
			.locator('[data-player-state] .pill', { hasText: 'Ready' })
			.waitFor({ timeout: 30_000 })
			.then(() => true, () => false);
		ok('this device’s player starts by itself', ready);

		// Play the first album on this device.
		await go(page, '#/albums');
		const album = page.locator('a[href^="#/album/"]').first();
		await album.waitFor({ timeout: 15_000 });
		await album.click();
		const play = page.locator('[data-page-play]').first();
		await play.waitFor({ timeout: 15_000 });
		await play.click();
		const started = await page
			.waitForFunction(() => [...document.querySelectorAll('audio')].some((a) => !a.paused && a.currentTime > 0.5), null, { timeout: 30_000 })
			.then(() => true)
			.catch(() => false);
		ok('a track starts playing in a media element', started);
		const t0 = await page.evaluate(position);
		await page.waitForTimeout(PLAY_SECONDS * 1000);
		const t1 = await page.evaluate(position);
		ok(`the position advances over ${PLAY_SECONDS} s`, t1 - t0 > PLAY_SECONDS * 0.7, `${t0.toFixed(1)} s to ${t1.toFixed(1)} s`);
		await go(page, '#/settings/advanced');
		await page.locator('[data-diag="mode"]').waitFor({ timeout: 10_000 }).catch(() => {});
		const active = ((await page.locator('[data-diag="mode"]').textContent().catch(() => '')) ?? '').trim();
		ok('the running player reports the media element', active === 'Media element', active);

		// Downloads without navigator.storage: the pages render, and an album is stored.
		await go(page, '#/settings/downloads');
		const persist = page.locator('[data-testid="persist-status"]');
		await persist.waitFor({ timeout: 10_000 }).catch(() => {});
		ok('the download settings render', (await persist.count()) === 1, ((await persist.textContent().catch(() => '')) ?? '').trim());
		await go(page, '#/albums');
		await page.locator('a[href^="#/album/"]').first().click();
		const dl = page.locator('[data-download]').first();
		if (await dl.waitFor({ timeout: 10_000 }).then(() => true, () => false)) {
			await dl.click();
			const stored = await page.locator('[data-downloaded]').first().waitFor({ timeout: 90_000 }).then(() => true, () => false);
			ok('an album downloads into the browser', stored);
		} else {
			ok('an album downloads into the browser', false, 'no Download button');
		}
		await go(page, '#/downloads');
		await page.waitForTimeout(1500);
		ok('the downloads page renders', ((await page.locator('main').textContent()) ?? '').length > 0);

		// Stop this device's player again.
		await go(page, '#/settings/advanced');
		const stop = page.locator('[data-player-stop]');
		if (await stop.count()) await stop.click();

		ok('no unexpected console or page errors', errors.length === 0, errors.join(' | ').slice(0, 2000));
	} catch (e) {
		ok('the check ran to the end', false, String(e?.stack ?? e));
	} finally {
		await browser.close();
	}
	return results;
}

let failed = 0;
for (const b of BROWSERS) {
	console.log(`${b}: ${BASE}`);
	const r = await check(b);
	const bad = r.filter((x) => !x.pass).length;
	failed += bad;
	console.log(`${b}: ${r.length - bad} of ${r.length} passed`);
}
process.exit(failed ? 1 : 0);
