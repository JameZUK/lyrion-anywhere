// Documentation screenshots of the web app, taken against the demo (tools/demo/run.sh): an
// invented library, generated artwork, made-up players and invented listening history, so
// nothing from a real installation appears.
//
//   tools/demo/shots.sh                        # builds nothing; runs the demo and this script
//   node tools/demo/screenshots.mjs [out-dir]  # against a demo already running; default docs/images/web
//
// Needs playwright-core and a Chromium it can drive (npx playwright install chromium).
// Set PLAYWRIGHT_CORE to its path if it isn't resolvable from here. Every shot waits for its
// screen's real content (no spinners, "Loading…" or unloaded artwork) before it is taken.
// Most scenes control the demo's Lounge; the signal-path scenes play in the browser itself
// (through the gateway, which re-encodes to Opus when High is chosen), so they need ffmpeg
// with libopus where the demo runs.
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';

const require = createRequire(import.meta.url);
const pw = require(process.env.PLAYWRIGHT_CORE || 'playwright-core');
const BASE = process.env.DEMO_URL || 'http://127.0.0.1:19443';
const LMS = process.env.DEMO_LMS_URL || 'http://127.0.0.1:19000';
const USER = 'demo';
const PASSWORD = process.env.DEMO_PASSWORD || 'demo-password-123';
const OUT = process.argv[2] || 'docs/images/web';
const ONLY = process.env.ONLY ? new Set(process.env.ONLY.split(',')) : null;
const PLAYER = 'aa:bb:cc:00:00:01'; // the demo's "Lounge" (in sync with the Kitchen)
mkdirSync(OUT, { recursive: true });

const PHONE = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true };
const DESKTOP = { viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 };
// Artist ids in the demo library (alphabetical).
const PAPER_LANTERNS = 11;
// The quality badge on the demo's "Night Ferry" (a 24-bit/96 kHz FLAC release).
const HIRES_BADGE = 'Hi-Res · FLAC 24/96';

// This browser's own player starts without a tap (the signal-path scenes).
const browser = await pw.chromium.launch({ headless: true, args: ['--autoplay-policy=no-user-gesture-required'] });
const failures = [];

/** A signed-in page. `player`: the player it controls, or 'device' for this browser's own. */
async function session({ device, scheme, theme = 'material', accent = '#c2410c', signIn = true, player = PLAYER }) {
  const ctx = await browser.newContext({ ...device, colorScheme: scheme, locale: 'en-GB', timezoneId: 'Europe/London', serviceWorkers: 'block' });
  await ctx.addInitScript(installPending);
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`);
  await page.evaluate(
    ([p, prefs]) => {
      if (p !== 'device') {
        // Control the Lounge (Settings › This device), so the shots show the demo's rooms.
        localStorage.setItem('anywhere.player', p);
        localStorage.setItem('anywhere.defaultPlayer.demo', JSON.stringify({ kind: 'player', id: p }));
      }
      localStorage.setItem('anywhere.appearance', JSON.stringify(prefs));
      localStorage.setItem('anywhere.playback.mode', 'worklet'); // no self-test noise
    },
    [player, { theme, mode: 'auto', accent }],
  );
  await page.reload();
  if (signIn) {
    await page.fill('input[autocomplete=username]', USER);
    await page.fill('input[autocomplete=current-password]', PASSWORD);
    await page.click('button[type=submit]');
    await page.waitForSelector('main.content');
    await settle(page);
  }
  return { ctx, page };
}

/** Wait for real content: `ready` visible, then nothing loading and every visible image in. */
async function settle(page, ready, extra = 600) {
  if (ready) await page.locator(ready).first().waitFor({ state: 'visible', timeout: 20000 });
  try {
    await page.waitForFunction(() => !pending(), null, { timeout: 20000, polling: 200 });
  } catch (e) {
    throw new Error(`${await page.evaluate(() => pending())} still pending at ${page.url()}`);
  }
  await page.waitForTimeout(extra);
}

/** What a screen is still waiting for, or '' (installed in every page). */
function installPending() {
  globalThis.pending = () => {
    const shown = (e) => {
      const r = e.getBoundingClientRect();
      return r.width > 1 && r.height > 1 && r.bottom > 0 && r.top < innerHeight && getComputedStyle(e).visibility !== 'hidden';
    };
    const loading = [...document.querySelectorAll('body *')].find(
      (e) => e.childElementCount === 0 && /^(Loading|Searching)(…|\.\.\.)?$/.test((e.textContent || '').trim()) && shown(e),
    );
    if (loading) return `"${loading.textContent.trim()}"`;
    const spinning = [...document.querySelectorAll('.spin, .spinner, [aria-busy="true"]')].find(shown);
    if (spinning) return `a spinner (${spinning.className})`;
    const image = [...document.images].filter(shown).find((i) => !(i.complete && i.naturalWidth > 0));
    if (image) return `an image (${image.src.slice(0, 80)})`;
    return '';
  };
}

/** Go to an in-app route the way a link does (the router handles it; no page reload, so
 * what this device did, such as a Play next, is still known). */
async function go(page, hash, ready) {
  await page.evaluate((h) => {
    const a = document.createElement('a');
    a.href = h;
    document.body.append(a);
    a.click();
    a.remove();
  }, hash);
  await page.waitForTimeout(300);
  await settle(page, ready);
}

async function shot(page, name) {
  if (ONLY && !ONLY.has(name)) return;
  await page.screenshot({ path: `${OUT}/${name}.png` });
  console.log('wrote', `${OUT}/${name}.png`);
}

/** Run one group of shots; a failure is reported at the end without stopping the rest. */
async function group(names, fn) {
  if (ONLY && !names.some((n) => ONLY.has(n))) return;
  try {
    await fn();
  } catch (e) {
    failures.push(`${names.join(', ')}: ${e.message.split('\n')[0]}`);
    console.error('FAILED', names.join(', '), e.message);
  }
}

/** Scroll the page so `selector` sits near the top (below `offset` pixels). */
async function scrollTo(page, selector, offset = 24) {
  await page.evaluate(
    ([sel, off]) => {
      const el = typeof sel === 'string' ? document.querySelector(sel) : null;
      el?.scrollIntoView({ block: 'start' });
      const main = document.querySelector('main.content');
      if (main && main.scrollHeight > main.clientHeight) main.scrollBy(0, -off);
      else scrollBy(0, -off);
    },
    [selector, offset],
  );
  await page.waitForTimeout(400);
}

/** The year whose story is fullest: this year, or last year early in January–February. */
function storyYear() {
  const now = new Date();
  return now.getMonth() < 2 ? now.getFullYear() - 1 : now.getFullYear();
}

async function pickYear(page) {
  const y = storyYear();
  const label = y === new Date().getFullYear() ? 'This year' : String(y);
  await page.selectOption('select[data-period]', { label });
  // Album and track titles arrive after the numbers.
  await page.waitForFunction(() => !document.body.innerText.includes('no longer in the library'), null, { timeout: 20000 });
  await settle(page, '[data-open-story]', 800);
}

/** Wait until `selector`'s text contains `text` (the quality badge, the signal path's verdict). */
async function waitForText(page, selector, text, timeout = 30000) {
  await page.waitForFunction(([sel, t]) => [...document.querySelectorAll(sel)].some((e) => e.textContent.includes(t)), [selector, text], {
    timeout,
    polling: 250,
  });
}

/**
 * The signal path of this browser's own stream re-encoded by the gateway: the browser plays the
 * demo's "Night Ferry" (FLAC 24/96), the badge opens the sheet, High is chosen on the gateway
 * block, and the stream starts again as Opus 160 (the verdict says Transcoded, in amber).
 */
async function transcodedPath(page) {
  await waitForText(page, '[data-quality-badge]', HIRES_BADGE);
  await settle(page, null, 1500);
  await page.locator('[data-quality-badge]').first().click();
  await settle(page, '[data-quality-switch]');
  await page.click('[data-quality-switch] [data-level="high"]');
  await waitForText(page, '[data-signal-path] .verdict', 'Transcoded · Opus 160');
  await waitForText(page, '[data-signal-path] [data-origin]', 'High');
  // The mini player's chip says the verdict word alone.
  await waitForText(page, '[data-mini-quality]', 'Transcoded');
  await settle(page, null, 1000);
}

// --- Phone, signed out -----------------------------------------------------------------

await group(['sign-in-phone'], async () => {
  const { ctx, page } = await session({ device: PHONE, scheme: 'dark', accent: '#6750a4', signIn: false });
  await settle(page, 'input[autocomplete=username]');
  await page.fill('input[autocomplete=username]', USER);
  await page.evaluate(() => document.activeElement?.blur());
  await shot(page, 'sign-in-phone');
  await ctx.close();
});

// --- Desktop, light --------------------------------------------------------------------

await group(['now-playing-desktop', 'library-desktop', 'album-desktop', 'artist-desktop', 'stats-desktop', 'rooms-desktop'], async () => {
  const { ctx, page } = await session({ device: DESKTOP, scheme: 'light' });
  await go(page, '#/', '[data-page-tab], .np, main.content');
  await page.waitForSelector('canvas, [data-waveform], .wave', { timeout: 10000 }).catch(() => {});
  await waitForText(page, '[data-quality-badge]', HIRES_BADGE);
  await settle(page, null, 1500);
  await shot(page, 'now-playing-desktop');

  await go(page, '#/library', 'main.content img');
  await shot(page, 'library-desktop');

  await go(page, '#/album/1', 'text=Review');
  await shot(page, 'album-desktop');

  await go(page, `#/artist/${PAPER_LANTERNS}`, 'text=Biography');
  await page.getByRole('button', { name: 'Read more' }).first().click().catch(() => {});
  await settle(page);
  await shot(page, 'artist-desktop');

  await go(page, '#/stats', 'select[data-period]');
  await pickYear(page);
  await shot(page, 'stats-desktop');

  await go(page, '#/rooms', 'text=Garden Room');
  await shot(page, 'rooms-desktop');
  await ctx.close();
});

// --- Desktop, dark ---------------------------------------------------------------------

await group(['now-playing-desktop-dark'], async () => {
  const { ctx, page } = await session({ device: DESKTOP, scheme: 'dark', accent: '#1f5fbf' });
  await go(page, '#/', 'main.content');
  await waitForText(page, '[data-quality-badge]', HIRES_BADGE);
  await settle(page, null, 1500);
  await shot(page, 'now-playing-desktop-dark');
  await ctx.close();
});

// --- Admin (the LMS plugin's settings page, served by the demo LMS) ----------------------

await group(['admin-desktop'], async () => {
  const ctx = await browser.newContext({ ...DESKTOP, colorScheme: 'light', locale: 'en-GB', timezoneId: 'Europe/London' });
  await ctx.addInitScript(installPending);
  const page = await ctx.newPage();
  await page.goto(`${LMS}/demo/admin`);
  await settle(page, '#aw-devices table');
  await scrollTo(page, '#aw-h-users', 12);
  await shot(page, 'admin-desktop');
  await ctx.close();
});

// --- Phone, light (before the dark set's Play next changes the queue) ----------------------

await group(['now-playing-phone-light'], async () => {
  const { ctx, page } = await session({ device: PHONE, scheme: 'light' });
  await go(page, '#/', '[data-np-phone]');
  await waitForText(page, '[data-quality-badge]', HIRES_BADGE);
  await settle(page, null, 1500);
  await shot(page, 'now-playing-phone-light');
  await ctx.close();
});

// --- Phone, dark -----------------------------------------------------------------------

await group(['now-playing-phone', 'play-on-phone', 'lyrics-phone', 'home-phone', 'library-phone', 'search-phone', 'queue-phone', 'year-story-phone', 'settings-phone'], async () => {
  const { ctx, page } = await session({ device: PHONE, scheme: 'dark', accent: '#6750a4' });
  await go(page, '#/', '[data-np-phone]');
  await waitForText(page, '[data-quality-badge]', HIRES_BADGE);
  await settle(page, null, 1500);
  await shot(page, 'now-playing-phone');

  await page.click('[data-play-on]');
  await settle(page, 'text=Kitchen', 900);
  await shot(page, 'play-on-phone');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(500);
  if (await page.locator('[data-play-on][aria-expanded="true"]').count()) await page.click('[data-play-on]');

  await page.click('[data-page-tab="lyrics"]');
  await settle(page, '[data-lyrics-pane]', 1500);
  await shot(page, 'lyrics-phone');
  await page.click('[data-page-tab="cover"]').catch(() => {});

  await go(page, '#/home', 'text=Recently played');
  await settle(page, null, 1200);
  await shot(page, 'home-phone');

  await go(page, '#/library', 'main.content img');
  await shot(page, 'library-phone');

  await go(page, '#/search', '[data-search-input]');
  await page.fill('[data-search-input]', 'hour');
  await settle(page, '[data-group]', 1200);
  await page.evaluate(() => document.activeElement?.blur());
  await settle(page);
  await shot(page, 'search-phone');

  // Play next from this device makes the queue's "Up next" section.
  await go(page, '#/album/7', '[data-track-more]');
  for (const n of [1, 3]) {
    await page.locator('[data-track-more]').nth(n).click();
    await page.getByRole('menuitem', { name: 'Play next' }).click();
    await page.waitForTimeout(1500);
  }
  await go(page, '#/queue', 'h2.section');
  await shot(page, 'queue-phone');

  await go(page, '#/stats', 'select[data-period]');
  await pickYear(page);
  await page.click('[data-open-story]');
  await settle(page, '[data-year-story]', 800);
  for (let i = 0; i < 8 && !(await page.locator('[data-story-card="five"]').isVisible()); i++) {
    await page.click('[data-story-next]');
    await page.waitForTimeout(700);
  }
  await settle(page, null, 1200);
  await shot(page, 'year-story-phone');
  await page.click('[data-story-close]');

  // Settings is an index (Wave 3.5): one screen of rows with their summaries.
  await go(page, '#/settings', '[data-settings-index]');
  await settle(page, null, 600);
  await shot(page, 'settings-phone');
  await ctx.close();
});

// --- This browser playing (last: its player leaves the demo's list when it closes) --------

await group(['signal-path-phone'], async () => {
  const { ctx, page } = await session({ device: PHONE, scheme: 'dark', accent: '#6750a4', player: 'device' });
  await go(page, '#/', '[data-np-phone]');
  await transcodedPath(page);
  await shot(page, 'signal-path-phone');
  await ctx.close();
});

// On a desktop the signal-path sheet is a dialog up to 760 px wide: the blocks sit in a row
// with arrows, and the quality switch is on its own line beneath them.
await group(['signal-path-desktop'], async () => {
  const { ctx, page } = await session({ device: DESKTOP, scheme: 'light', player: 'device' });
  await go(page, '#/', 'main.content');
  await transcodedPath(page);
  await shot(page, 'signal-path-desktop');
  await ctx.close();
});

await browser.close();
if (failures.length) {
  console.error(`\n${failures.length} group(s) failed:\n  ${failures.join('\n  ')}`);
  process.exit(1);
}
