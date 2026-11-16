#!/usr/bin/env node
// End-to-end check of two-step verification in the web app, in a real browser through
// Playwright: every authenticator-code field is typed into and every button really clicked,
// so the browser's own form checks run. It needs a built gateway, a built web app, python3
// with Pillow (for tools/demo/fake_lms.py) and Playwright with a browser installed. It starts
// the demo LMS and the gateway itself, on loopback, with a throwaway data directory and user.
// CI runs it (the `web` job); by hand:
//
//   GATEWAY=target/debug/anywhere-gateway WEB=web/app/build \
//     PLAYWRIGHT_DIR=web/e2e/node_modules/playwright node web/app/tests/e2e/two-step.mjs
//
// It signs in, turns two-step verification on in Settings › Account (reading the setup key
// off the page, as someone typing it into an authenticator would), signs out, and signs in
// again: a malformed code is stopped by the field, a wrong one is refused by the server, the
// right one signs in. Then it changes the password with a code, signs in with the new
// password, turns two-step verification off with a code, and signs in without one. Exit
// status 1 if a check fails. Codes are single use, so it waits for the next 30-second step
// between some of them: about a minute and a half in all.
//
// Why it exists: the sign-in form's code field was written `pattern="[0-9]{6}"`, which Svelte
// compiles to `[0-9]6`, so browsers refused every code ("Match the requested format") from
// the first release to 0.8.0. The component tests dispatch `submit` themselves and jsdom
// does not apply `pattern` before it, so only a browser shows this.
//
// PORT (default 19811) is the gateway's; its admin port is PORT+2 and the demo LMS PORT+4.
// BROWSER (default chromium). HEADED=1 shows the browser.
import { spawn, spawnSync } from 'node:child_process';
import { createHmac, randomBytes } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const env = process.env;
const GATEWAY = env.GATEWAY ?? '';
const WEB = resolve(env.WEB ?? 'web/app/build');
const PORT = Number(env.PORT ?? 19811);
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
const USER = 'two-step-check';
const PASSWORD = randomBytes(12).toString('base64url');
const NEW_PASSWORD = randomBytes(12).toString('base64url');
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
const check = (ok, what) => {
	console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${what}`);
	if (!ok) failures++;
};

// --- Authenticator codes (RFC 6238: HMAC-SHA1, 30-second steps, six digits) ---------------
const STEP_MS = 30_000;
function base32(text) {
	const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
	let bits = '';
	for (const c of text.replace(/[\s=]/g, '').toUpperCase()) {
		const v = A.indexOf(c);
		if (v < 0) throw new Error('the setup key is not base32');
		bits += v.toString(2).padStart(5, '0');
	}
	return Buffer.from((bits.match(/.{8}/g) ?? []).map((b) => parseInt(b, 2)));
}
function codeAt(secret, step) {
	const counter = Buffer.alloc(8);
	counter.writeBigUInt64BE(BigInt(step));
	const mac = createHmac('sha1', secret).update(counter).digest();
	const at = mac[19] & 15;
	return String((mac.readUInt32BE(at) & 0x7fffffff) % 1_000_000).padStart(6, '0');
}
const stepNow = () => Math.floor(Date.now() / STEP_MS);
let lastStep = 0;
/** A code the gateway has not seen: each is single use and must be newer than the last. The
 *  gateway takes one step either side of its clock, so the step after this one is usable now. */
async function freshCode(secret) {
	const step = Math.max(stepNow(), lastStep + 1);
	if (step > stepNow() + 1) {
		const ms = (step - 1) * STEP_MS - Date.now() + 1000;
		console.log(`  (waiting ${Math.ceil(ms / 1000)} s for the next code)`);
		await wait(ms);
	}
	lastStep = step;
	return codeAt(secret, step);
}
/** Six digits that are no code near now. */
function wrongCode(secret) {
	const near = new Set([-2, -1, 0, 1, 2, 3].map((d) => codeAt(secret, stepNow() + d)));
	for (let n = 0; ; n++) {
		const c = String(n).padStart(6, '0');
		if (!near.has(c)) return c;
	}
}

// --- The demo LMS and the gateway ---------------------------------------------------------
const data = mkdtempSync(join(tmpdir(), 'two-step-check-'));
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
	page.setDefaultTimeout(15_000);
	const errors = [];
	page.on('pageerror', (e) => errors.push(`pageerror: ${e}`));
	let logins = 0;
	page.on('request', (r) => {
		if (r.method() === 'POST' && new URL(r.url()).pathname === '/v1/auth/login') logins++;
	});

	const LOGIN_CODE = 'form.card input[autocomplete=one-time-code], main input[autocomplete=one-time-code]';
	const signedIn = () => page.waitForSelector('main.content', { timeout: 20_000 }).then(() => true, () => false);
	const atSignIn = () => page.locator('input[autocomplete=username]').isVisible();
	const alertText = async () => ((await page.locator('[role=alert]').first().textContent().catch(() => '')) ?? '').trim();
	/** Does the browser's own check pass this field as it stands? */
	const valid = (locator) => locator.evaluate((el) => el.checkValidity());

	/** Username and password, and the button. */
	async function credentials(password) {
		await page.waitForSelector('input[autocomplete=username]');
		await page.fill('input[autocomplete=username]', USER);
		await page.fill('input[autocomplete=current-password]', password);
		await page.click('button[type=submit]');
	}
	async function account() {
		await page.evaluate(() => {
			location.hash = '#/settings/account';
		});
		await page.waitForSelector('[data-account-row=two-step]:not([disabled])');
	}
	async function signOut() {
		await account();
		await page.getByRole('button', { name: 'Sign out', exact: true }).click();
		await page.waitForSelector('input[autocomplete=username]');
	}
	const twoStepValue = async () => ((await page.locator('[data-account-row=two-step] .value').textContent()) ?? '').trim();
	const closeSheet = () => page.locator('dialog[open] button[aria-label=Close]').click();

	// 1. Sign in with the password alone, and turn two-step verification on.
	console.log('sign in, and turn two-step verification on');
	await page.goto(BASE);
	await credentials(PASSWORD);
	check(await signedIn(), 'signed in with the password alone');
	await account();
	check((await twoStepValue()) === 'Off', 'Settings › Account shows two-step verification Off');
	await page.click('[data-account-row=two-step]');
	const sheet = page.locator('[data-two-step]');
	await sheet.locator('input[type=password]').fill(PASSWORD);
	await sheet.getByRole('button', { name: 'Turn on' }).click();
	const key = (await sheet.locator('code.key').textContent()) ?? '';
	const secret = base32(key);
	check(secret.length >= 16, 'the setup key is on the page');
	const setupCode = sheet.locator('input[autocomplete=one-time-code]');
	await setupCode.fill('12345');
	check(!(await valid(setupCode)), 'five digits do not pass the field');
	// As an authenticator app shows it: two groups of three.
	const first = await freshCode(secret);
	await setupCode.fill(`${first.slice(0, 3)} ${first.slice(3)}`);
	check(await valid(setupCode), 'a code typed with its space passes the field');
	await sheet.getByRole('button', { name: 'Confirm' }).click();
	await page.waitForFunction(() => /is on\./.test(document.querySelector('[data-two-step-status]')?.textContent ?? ''));
	check(true, 'the code is accepted: two-step verification is on');
	await closeSheet();
	check((await twoStepValue()) === 'On', 'Settings › Account shows On');

	// 2. Sign out, and sign in with a code: the regression.
	console.log('sign in with a code');
	await signOut();
	await credentials(PASSWORD);
	const code = page.locator(LOGIN_CODE).first();
	await code.waitFor();
	check((await alertText()) === 'Enter the code from your authenticator app.', 'the form asks for a code');

	let before = logins;
	await code.fill('12345a');
	check(!(await valid(code)), 'a malformed code does not pass the field');
	await page.click('button[type=submit]');
	await wait(1000);
	check(logins === before && (await atSignIn()), 'and the browser sends nothing for it');

	before = logins;
	await code.fill(wrongCode(secret));
	check(await valid(code), 'six digits pass the field');
	await page.click('button[type=submit]');
	await page.waitForFunction(() => /not right/.test(document.querySelector('[role=alert]')?.textContent ?? ''));
	check(logins === before + 1 && (await atSignIn()), `a wrong code reaches the server and is refused (${await alertText()})`);

	await code.fill(await freshCode(secret));
	check(await valid(code), 'the right code passes the field');
	await page.click('button[type=submit]');
	check(await signedIn(), 'the right code signs in');

	// 3. Change the password, which asks for a code too.
	console.log('change the password with a code');
	await account();
	await page.click('[data-account-row=password]');
	const form = page.locator('[data-change-password]');
	await form.locator('[data-current]').fill(PASSWORD);
	await form.locator('[data-new]').fill(NEW_PASSWORD);
	await form.locator('[data-confirm]').fill(NEW_PASSWORD);
	const changeCode = form.locator('[data-code]');
	check(await changeCode.isVisible(), 'the form asks for a code');
	await changeCode.fill(await freshCode(secret));
	check(await valid(changeCode), 'the code passes the field');
	await form.locator('[data-change-submit]').click();
	check(await page.waitForSelector('[data-change-done]').then(() => true, () => false), 'the password is changed');
	await closeSheet();

	await signOut();
	await credentials(PASSWORD);
	await page.waitForFunction(() => /not right/.test(document.querySelector('[role=alert]')?.textContent ?? ''));
	check(await atSignIn(), 'the old password no longer signs in');
	await credentials(NEW_PASSWORD);
	await page.locator(LOGIN_CODE).first().waitFor();
	await page.locator(LOGIN_CODE).first().fill(await freshCode(secret));
	await page.click('button[type=submit]');
	check(await signedIn(), 'the new password and a code sign in');

	// 4. Turn two-step verification off, and sign in without a code.
	console.log('turn two-step verification off');
	await account();
	await page.click('[data-account-row=two-step]');
	await sheet.locator('input[type=password]').fill(NEW_PASSWORD);
	const offCode = sheet.locator('input[autocomplete=one-time-code]');
	await offCode.fill(await freshCode(secret));
	check(await valid(offCode), 'the code passes the field');
	await sheet.getByRole('button', { name: 'Turn off' }).click();
	await page.waitForFunction(() => /is off\./.test(document.querySelector('[data-two-step-status]')?.textContent ?? ''));
	check(true, 'two-step verification is off');
	await closeSheet();
	check((await twoStepValue()) === 'Off', 'Settings › Account shows Off');
	await signOut();
	await credentials(NEW_PASSWORD);
	check(await signedIn(), 'the password alone signs in again');

	check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join(' | ')}` : ''}`);
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
