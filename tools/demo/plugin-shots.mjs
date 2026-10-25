// Documentation screenshots of the LMS plugin's settings page (docs/images/plugin), taken
// against a throwaway stock LMS in Docker with the released plugin installed, so nothing from
// a real installation appears: example values only (music.example.com, 10.8.0.0/24, alice).
//
// 1. Start the LMS (loopback ports only; never host networking):
//
//      S=$(mktemp -d)   # scratch: config, music
//      mkdir -p $S/config/prefs $S/config/cache/InstalledPlugins/Plugins $S/music
//      cp -a <unzipped plugin release> $S/config/cache/InstalledPlugins/Plugins/LyrionAnywhere
//      ffmpeg -f lavfi -i sine=frequency=440:duration=20 -metadata title='Test Tone' $S/music/tone.flac
//      printf -- '---\nwizardDone: 1\nmediadirs:\n  - /music\n' > $S/config/prefs/server.prefs
//      docker run -d --name la-shots-lms -p 127.0.0.1:29000:9000 -p 127.0.0.1:23483:3483 \
//        -p 127.0.0.1:29090:9090 -e PUID=$(id -u) -e PGID=$(id -g) \
//        -v $S/config:/config -v $S/music:/music:ro lmscommunity/lyrionmusicserver:9.1.1
//
// 2. Once http://127.0.0.1:29000/ answers:
//
//      node tools/demo/plugin-shots.mjs [out-dir]    # default docs/images/plugin
//      python3 tools/demo/optimise_png.py docs/images/plugin/*.png   # or oxipng
//
// 3. docker rm -f la-shots-lms; rm -rf $S
//
// The script enables the gateway (Public URL https://music.example.com), adds the user alice
// through the page, and signs in a device for her from inside the container (curl and perl
// there, so the device's address is 127.0.0.1) whose player socket omits "protocol": the
// gateway closes it with 4426 and records protocol 1, so the device is tagged "needs
// updating". Needs playwright (or playwright-core) with a Chromium: set PLAYWRIGHT to its
// path if it isn't resolvable from here. LMS_URL and LMS_CONTAINER override the defaults.
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const require = createRequire(import.meta.url);
const pw = require(process.env.PLAYWRIGHT || 'playwright');
const LMS = process.env.LMS_URL || 'http://127.0.0.1:29000';
const CONTAINER = process.env.LMS_CONTAINER || 'la-shots-lms';
const PAGE = `${LMS}/plugins/LyrionAnywhere/settings/basic.html`;
const OUT = process.argv[2] || 'docs/images/plugin';
const USER = 'alice';
const PASSWORD = 'example-password-123';
const DEVICE = 'Alice’s phone';
const MARGIN = 8;
mkdirSync(OUT, { recursive: true });

const browser = await pw.chromium.launch({ headless: true });
// Tall, so LMS's scrolling content panel holds the whole page.
const ctx = await browser.newContext({ viewport: { width: 1100, height: 4000 }, deviceScaleFactor: 1, colorScheme: 'light', locale: 'en-GB', timezoneId: 'Europe/London' });
const page = await ctx.newPage();

async function open() {
  await page.goto(PAGE);
  await page.waitForFunction(() => !/Loading/.test(document.getElementById('aw-status')?.textContent || 'Loading'));
}

async function gatewayRunning() {
  return page.evaluate(() => /running/i.test(document.getElementById('aw-status').textContent) && !document.querySelector('#aw-create button[disabled], #aw-create-btn[disabled]'));
}

// --- set-up: the gateway on, a user, a device on an old app ----------------------------------

await open();
if (!(await gatewayRunning()) || (await page.inputValue('#pref_public_url')) !== 'https://music.example.com') {
  await page.check('#pref_enabled');
  await page.fill('#pref_listen', '127.0.0.1:9443');
  await page.fill('#pref_public_url', 'https://music.example.com');
  await page.fill('#pref_trusted_networks', '');
  await Promise.all([page.waitForNavigation(), page.evaluate(() => document.settingsForm.submit())]);
  for (let i = 0; i < 60 && !(await gatewayRunning()); i++) {
    await page.waitForTimeout(1000);
    await open();
  }
  if (!(await gatewayRunning())) throw new Error('the gateway did not start');
}

await page.waitForSelector('#aw-users table, #aw-users p');
if (!(await page.locator('#aw-users td', { hasText: new RegExp(`^${USER}$`) }).count())) {
  await page.fill('#aw-new-user', USER);
  await page.fill('#aw-new-password', PASSWORD);
  await page.selectOption('#aw-new-role', 'listener');
  await page.click('#aw-create-btn');
  await page.locator('#aw-users td', { hasText: new RegExp(`^${USER}$`) }).waitFor();
}

const devices = await page.evaluate(() => document.getElementById('aw-devices').textContent);
if (!devices.includes('needs updating')) {
  const login = JSON.parse(execFileSync('docker', [
    'exec', CONTAINER, 'curl', '-sf', '-A', 'okhttp/4.12.0',
    '-H', 'Content-Type: application/json', '-H', 'X-Anywhere-Client: 1',
    '-d', JSON.stringify({ username: USER, password: PASSWORD, device_name: DEVICE, client_kind: 'android', client_platform: 'Android 15' }),
    'http://127.0.0.1:9443/v1/auth/login',
  ]).toString());
  // A player socket from an app before protocol 2: {"token"} alone. Prints the close code.
  const ws = `
    use IO::Socket::INET;
    my $s = IO::Socket::INET->new(PeerAddr => '127.0.0.1:9443') or die "connect: $!";
    print $s "GET /v1/player HTTP/1.1\\r\\nHost: 127.0.0.1:9443\\r\\nUpgrade: websocket\\r\\nConnection: Upgrade\\r\\n"
      . "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\\r\\nSec-WebSocket-Version: 13\\r\\nUser-Agent: okhttp/4.12.0\\r\\n\\r\\n";
    my $h = ''; while ($h !~ /\\r\\n\\r\\n/) { sysread($s, my $c, 1) or die 'no upgrade'; $h .= $c }
    $h =~ m{^HTTP/1.1 101} or die "refused: $h";
    my $p = '{"token":"' . $ARGV[0] . '"}'; my @m = (1, 2, 3, 4);
    my $len = length $p; my $f = pack('C', 0x81) . ($len < 126 ? pack('C', 0x80 | $len) : pack('Cn', 0x80 | 126, $len));
    $f .= pack('C4', @m); my $i = 0; $f .= join '', map { chr(ord($_) ^ $m[$i++ % 4]) } split //, $p;
    print $s $f;
    sysread($s, my $r, 64); my ($op, $l, $code) = unpack('CCn', $r); print $code, "\\n";`;
  const code = execFileSync('docker', ['exec', CONTAINER, 'perl', '-e', ws, login.access_token]).toString().trim();
  if (code !== '4426') throw new Error(`player socket closed with ${code}, not 4426`);
  await open();
}

// --- shots ---------------------------------------------------------------------------------

/** The settings block titled `title` (a `[% WRAPPER setting %]`). */
const section = (title) => page.locator('.settingSection', { has: page.locator('.prefHead', { hasText: new RegExp(`^${title}$`) }) });

async function shot(name, from, to = from) {
  const a = await from.boundingBox();
  const b = await to.boundingBox();
  const x = 0;
  // Never above the content panel (LMS's plugin chooser bar is not part of the page).
  const top = (await page.locator('#content').boundingBox()).y;
  const y = Math.max(top, Math.min(a.y, b.y) - MARGIN);
  const bottom = Math.max(a.y + a.height, b.y + b.height) + MARGIN;
  await page.mouse.move(0, 3990); // no hover effects
  await page.screenshot({ path: `${OUT}/${name}.png`, clip: { x, y, width: 1100, height: bottom - y } });
  console.log(`${OUT}/${name}.png`);
}

/** Replace a field's value as typing would, so the page's input handlers run. */
async function type(selector, value) {
  await page.fill(selector, '');
  await page.type(selector, value);
}

// 1. The top of the page: the logo and introduction, Gateway, Listen address, Public URL.
await open();
await shot('settings-overview', page.locator('.anywhere-logo'), section('Public URL'));

// 2. An http:// Public URL: the warning and the "Allow http:// anyway" box.
await type('#pref_public_url', 'http://10.8.0.1:9443');
await page.locator('#aw-http-warning').waitFor({ state: 'visible' });
await shot('public-url-http', section('Public URL'));

// 3. The VPN set-up: Listen address on the VPN interface, an http:// Public URL with its box
// ticked, and the VPN's subnet as a trusted network.
await open();
await type('#pref_listen', '10.8.0.1:9443');
await type('#pref_public_url', 'http://10.8.0.1:9443');
await page.check('#anywhere_allow_http');
await type('#pref_trusted_networks', '10.8.0.0/24');
await page.locator('#aw-trusted-warning').waitFor({ state: 'visible' });
await shot('trusted-networks', section('Listen address'), section('Trusted networks'));

// 4. Cloudflare Tunnel before anything is installed (Install is never pressed).
await open();
await page.waitForFunction(() => !/Loading/.test(document.getElementById('aw-tunnel-state').textContent));
await page.waitForTimeout(500);
const tunnelParts = ['#aw-h-tunnel', '#aw-tunnel', '#aw-tunnel-quick-box', '#aw-tunnel-token-form', '#aw-tunnel-create-form', '#aw-tunnel-log-box'];
const visible = [];
for (const s of tunnelParts) if (await page.locator(s).isVisible()) visible.push(s);
await shot('cloudflare-tunnel', page.locator(visible[0]), page.locator(visible[visible.length - 1]));

// 5. Users, with Add a device open for alice: the one-time code and its QR code.
await page.click(`button[aria-label="Add a device for ${USER}"]`);
await page.locator('#aw-pair .aw-qr svg').waitFor();
await page.locator('#aw-pair').evaluate((el) => el.querySelector('h4')?.blur());
await shot('users-add-device', page.locator('#aw-h-users'), page.locator('#aw-pair'));

// 6. Devices, with alice's phone on an app too old for this server.
await page.click(`button[aria-label="Done adding a device for ${USER}"]`);
await page.locator('#aw-devices .aw-tag', { hasText: 'needs updating' }).waitFor();
await shot('devices-needs-updating', page.locator('#aw-h-devices'), page.locator('#aw-devices'));

await browser.close();
