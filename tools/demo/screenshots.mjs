// Documentation screenshots, taken against the demo (tools/demo/run.sh): an invented library,
// generated artwork and made-up players, so nothing from a real installation appears.
//
//   tools/demo/run.sh &                       # in another terminal
//   node tools/demo/screenshots.mjs [out-dir] # default docs/images
//
// Needs playwright-core and a Chromium it can drive (npx playwright install chromium).
// Set PLAYWRIGHT_CORE to its path if it isn't resolvable from here.
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';

const require = createRequire(import.meta.url);
const pw = require(process.env.PLAYWRIGHT_CORE || 'playwright-core');
const BASE = process.env.DEMO_URL || 'http://127.0.0.1:19443';
const USER = 'demo';
const PASSWORD = process.env.DEMO_PASSWORD || 'demo-password-123';
const OUT = process.argv[2] || 'docs/images';
const PLAYER = 'aa:bb:cc:00:00:01'; // the demo's "Lounge"
mkdirSync(OUT, { recursive: true });

const PHONE = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true };
const DESKTOP = { viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 };

const browser = await pw.chromium.launch({ headless: true });

async function session({ device, scheme, theme, accent = '#c2410c', signIn = true }) {
  const ctx = await browser.newContext({ ...device, colorScheme: scheme });
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`);
  await page.evaluate(
    ([p, prefs]) => {
      localStorage.setItem('anywhere.player', p);
      // The demo has no audio server for this browser to play through: start on Lounge
      // (Settings › This device), so the shots show the "Playing on …" strip.
      localStorage.setItem('anywhere.defaultPlayer.demo', JSON.stringify({ kind: 'player', id: p }));
      localStorage.setItem('anywhere.appearance', JSON.stringify(prefs));
      localStorage.setItem('anywhere.playback.mode', 'worklet'); // no self-test noise
    },
    [PLAYER, { theme, mode: 'auto', accent }],
  );
  await page.reload();
  if (signIn) {
    await page.fill('input[autocomplete=username]', USER);
    await page.fill('input[autocomplete=current-password]', PASSWORD);
    await page.click('button[type=submit]');
    await page.waitForSelector('main.content');
    await page.waitForTimeout(1500);
  }
  return { ctx, page };
}

async function shot(page, name, hash, wait = 1500) {
  if (hash !== undefined) {
    await page.evaluate((h) => (location.hash = h), hash);
    await page.waitForTimeout(wait);
  }
  await page.screenshot({ path: `${OUT}/${name}.png` });
  console.log('wrote', `${OUT}/${name}.png`);
}

{
  const { ctx, page } = await session({ device: PHONE, scheme: 'dark', theme: 'material', signIn: false });
  await shot(page, 'sign-in-phone');
  await ctx.close();
}
{
  const { ctx, page } = await session({ device: DESKTOP, scheme: 'light', theme: 'material' });
  await shot(page, 'now-playing-desktop', '#/');
  await shot(page, 'album-desktop', '#/album/1');
  await shot(page, 'apps-desktop', '#/apps', 3500);
  await ctx.close();
}
{
  const { ctx, page } = await session({ device: DESKTOP, scheme: 'dark', theme: 'material', accent: '#1f5fbf' });
  await shot(page, 'albums-desktop-dark', '#/albums', 2500);
  await ctx.close();
}
{
  const { ctx, page } = await session({ device: PHONE, scheme: 'dark', theme: 'material', accent: '#6750a4' });
  await shot(page, 'now-playing-phone', '#/');
  await page.click('button[aria-label="More"]');
  await page.waitForTimeout(1200);
  await shot(page, 'more-menu-phone');
  await page.keyboard.press('Escape');
  await page.evaluate(() => (location.hash = '#/settings'));
  await page.waitForTimeout(800);
  await page.evaluate(() => {
    const h = [...document.querySelectorAll('h2')].find((e) => e.textContent?.trim() === 'Appearance');
    h?.scrollIntoView({ block: 'start' });
    scrollBy(0, -24);
  });
  await page.waitForTimeout(400);
  await shot(page, 'appearance-phone');
  await ctx.close();
}
{
  const { ctx, page } = await session({ device: PHONE, scheme: 'light', theme: 'contrast' });
  await shot(page, 'now-playing-phone-contrast', '#/');
  await ctx.close();
}
{
  const { ctx, page } = await session({ device: PHONE, scheme: 'light', theme: 'classic' });
  await shot(page, 'albums-phone-classic', '#/albums', 2500);
  await ctx.close();
}
await browser.close();
