// Lyrion Anywhere settings page: users, devices and activity, via `anywhere ...` commands.
//
// Names here (usernames, device names, audit details) can come from remote users, so the
// page is built with textContent only: never innerHTML or similar.
//
// This file is ASCII only: other characters are written as \u escapes, so the page shows
// them correctly however the file is served (t/10-web-files.t checks).
(function () {
  'use strict';

  // --- pure helpers (also used by the tests) ------------------------------------------

  /**
   * The JSON-RPC command for an operation: ["anywhere", op, "key:value", ..., "csrf:<token>"].
   * LMS refuses commands that change anything without the token, which only this page has
   * (LMS's JSON-RPC is otherwise open to any web page on the local network).
   */
  function command(op, args, csrf) {
    const out = ['anywhere', op];
    for (const [k, v] of Object.entries(args || {})) {
      if (k === 'csrf' || v === undefined || v === null || v === '') continue;
      out.push(k + ':' + String(v));
    }
    if (csrf) out.push('csrf:' + csrf);
    return out;
  }

  /** The token LMS put in the page (data-csrf on #anywhere-app), or ''. */
  function pageToken(doc) {
    const app = doc && doc.getElementById('anywhere-app');
    const t = app && app.getAttribute ? app.getAttribute('data-csrf') || '' : '';
    return /^[0-9a-f]{32}$/.test(t) ? t : '';
  }

  /** The `commands` argument for the plugins whose boxes are ticked: "a,b" (sorted), or ''. */
  function adminOnlyArg(rows) {
    const out = new Set();
    for (const r of rows) {
      if (!r.checked) continue;
      for (const c of Array.isArray(r.commands) ? r.commands : [r.command]) {
        if (typeof c === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(c)) out.add(c);
      }
    }
    return [...out].sort().join(',');
  }

  const DASH = '\u2013'; // en dash: "no value"
  const ELLIPSIS = '\u2026';
  const DOT = ' \u00b7 ';

  /** The gateway's default sign-in lifetime (accounts.rs DEFAULT_SESSION_DAYS). */
  const DEFAULT_SESSION_DAYS = 90;

  /** A sign-in lifetime in whole days (1 to 365), or null. */
  function validDays(v) {
    const s = String(v ?? '').trim();
    if (!/^\d{1,3}$/.test(s)) return null;
    const n = Number(s);
    return n >= 1 && n <= 365 ? n : null;
  }

  /**
   * The lifetime to show from the gateway's settings: its value (a number, or a numeric
   * string as LMS may pass it on), else its default, else ours.
   */
  function sessionDaysShown(s) {
    return validDays(s && s.session_days) ?? validDays(s && s.session_days_default) ?? DEFAULT_SESSION_DAYS;
  }

  /** A device name as the gateway takes it (1 to 64 characters, no control characters), trimmed, or null. */
  function validDeviceName(v) {
    if (typeof v !== 'string') return null;
    const s = v.trim();
    const n = [...s].length;
    if (n < 1 || n > 64 || /[\u0000-\u001f\u007f-\u009f]/.test(s)) return null;
    return s;
  }

  const CLIENT_KINDS = { pwa: 'Installed web app', browser: 'Browser tab', android: 'Android app' };

  /** What kind of client a device is, e.g. "Browser tab \u00b7 Safari on iPadOS". */
  function clientText(d) {
    const kind = d && Object.prototype.hasOwnProperty.call(CLIENT_KINDS, d.client_kind) ? CLIENT_KINDS[d.client_kind] : '';
    const platform = d && typeof d.client_platform === 'string' ? d.client_platform : '';
    return [kind, platform].filter(Boolean).join(DOT) || 'unknown';
  }

  function validUsername(u) {
    return /^[a-z0-9._-]{1,64}$/.test(u);
  }

  function validPassword(p) {
    return typeof p === 'string' && p.length >= 10 && p.length <= 1024 && !/[\u0000-\u001f\u007f]/.test(p);
  }

  function formatTime(unixSeconds) {
    if (!unixSeconds) return DASH;
    return new Date(unixSeconds * 1000).toLocaleString('en-GB');
  }

  function formatUptime(s) {
    if (!s) return DASH;
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
    return h ? `${h} h ${m} min` : `${m} min`;
  }

  /** Build an element. Text is always set as text, never parsed as HTML. */
  function el(tag, props, children) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (k === 'text') e.textContent = v;
      else if (k === 'onclick') e.addEventListener('click', v);
      else if (k === 'className') e.className = v;
      else e.setAttribute(k, v);
    }
    for (const c of children || []) e.append(c);
    return e;
  }

  /** Plain-text view of a diagnostics report (shown with textContent, never as HTML). */
  function diagnosticsText(r) {
    const when = r && r.updated_at ? `Reported ${formatTime(r.updated_at)}\n\n` : '';
    return when + JSON.stringify(r && r.report !== undefined ? r.report : r, null, 2);
  }

  const FORMAT_NAMES = { original: 'original', mp3: 'MP3', opus: 'Opus' };

  /** A user's download permission as text: "off", or "on (original, MP3, Opus)". */
  function downloadsText(u) {
    if (!u || u.downloads !== true) return 'off';
    const formats = (Array.isArray(u.download_formats) ? u.download_formats : [])
      .filter((f) => Object.prototype.hasOwnProperty.call(FORMAT_NAMES, f))
      .map((f) => FORMAT_NAMES[f]);
    return formats.length ? `on (${formats.join(', ')})` : 'on';
  }

  /** Time left on a pairing code, e.g. "9:05". */
  function formatCountdown(seconds) {
    const s = Math.max(0, Math.floor(Number(seconds) || 0));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  }

  /**
   * What the "Add a device" panel shows for a code from the gateway: the code grouped for
   * typing, the pairing link for the QR code (null unless the public URL is a usable https
   * address), and how many seconds it lasts. Null if the reply holds no valid code.
   * `qr` is the shared encoder (qr.js, globalThis.AnywhereQR).
   */
  function pairingView(data, publicUrl, qr) {
    const code = qr && data ? qr.normalisePairingCode(typeof data.code === 'string' ? data.code : '') : null;
    if (!code) return null;
    const secs = Number(data.expires_in);
    return {
      code: qr.groupPairingCode(code),
      link: qr.pairingLink(publicUrl, code),
      seconds: Number.isFinite(secs) && secs > 0 && secs <= 3600 ? Math.floor(secs) : 600,
      publicUrlSet: typeof publicUrl === 'string' && publicUrl.trim() !== '',
    };
  }

  const api = { command, adminOnlyArg, pageToken, validDays, sessionDaysShown, validDeviceName, clientText, validUsername, validPassword, formatTime, formatUptime, diagnosticsText, downloadsText, formatCountdown, pairingView, DEFAULT_SESSION_DAYS };
  if (typeof globalThis.__anywhereTest === 'object') {
    Object.assign(globalThis.__anywhereTest, api);
    return;
  }

  // --- LMS JSON-RPC ---------------------------------------------------------------------

  async function rpc(cmd) {
    const resp = await fetch('/jsonrpc.js', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: 1, method: 'slim.request', params: ['', cmd] }),
    });
    const body = await resp.json();
    return body.result || {};
  }

  // Sent with every command except status: harmless on read-only ones, required on the rest.
  let csrf = '';

  async function admin(op, args) {
    const r = await rpc(command(op, args, csrf));
    if (r.error) throw new Error(r.error);
    return r.data || {};
  }

  const $ = (id) => document.getElementById(id);

  function say(text, isError) {
    const m = $('aw-message');
    m.textContent = text;
    m.className = 'aw-message' + (isError ? ' aw-error' : '');
  }

  /** A button that needs a second click to confirm (no browser dialogs). */
  function confirmButton(label, confirmLabel, action) {
    const b = el('button', { type: 'button', text: label });
    let armed = false;
    b.addEventListener('click', async () => {
      if (!armed) {
        armed = true;
        b.textContent = confirmLabel;
        b.classList.add('aw-danger');
        setTimeout(() => {
          armed = false;
          b.textContent = label;
          b.classList.remove('aw-danger');
        }, 4000);
        return;
      }
      await action();
    });
    return b;
  }

  async function run(action, done) {
    try {
      await action();
      if (done) say(done);
      await refresh();
    } catch (e) {
      say(e.message, true);
    }
  }

  // --- views ------------------------------------------------------------------------------

  async function showStatus() {
    const s = await rpc(['anywhere', 'status']);
    const rows = [
      ['Gateway', s.running ? `running (pid ${s.pid})` : s.enabled ? 'not running' : 'disabled'],
      ['Up for', formatUptime(s.uptime)],
      ['Listening on', s.listen || DASH],
      ['Binary', s.binary_ok ? s.binary : `missing: ${s.binary || 'none for this platform'}`],
      ['Data', s.data_dir || DASH],
    ];
    if (s.error) rows.push(['Last error', s.error]);
    const box = $('aw-status');
    box.replaceChildren(
      el('dl', {}, rows.flatMap(([k, v]) => [el('dt', { text: k }), el('dd', { text: String(v) })])),
      el('button', { type: 'button', text: 'Restart gateway', onclick: () => run(async () => {
        const r = await rpc(command('restart', {}, csrf));
        if (r.error) throw new Error(r.error);
      }, 'Gateway restarted') }),
    );
    return s;
  }

  async function playerNames() {
    const r = await rpc(['players', 0, 200]);
    const names = new Map();
    for (const p of r.players_loop || []) names.set(p.playerid, p.name);
    return names;
  }

  async function showUsers(players) {
    const box = $('aw-users');
    const { users = [] } = await admin('users');
    if (!users.length) {
      box.replaceChildren(el('p', { className: 'aw-note', text: 'No users yet. Add the first one (as Admin) below.' }));
      return;
    }
    const table = el('table', { className: 'aw-table' }, [
      el('tr', {}, ['User', 'Role', 'Two-step', 'Status', 'Downloads', 'Home players', ''].map((h) => el('th', { text: h }))),
    ]);
    for (const u of users) {
      const grants = el('div', { className: 'aw-grants', text: ELLIPSIS });
      const actions = el('div', { className: 'aw-actions' }, [
        el('button', {
          type: 'button',
          text: u.disabled ? 'Enable' : 'Disable',
          onclick: () => run(() => admin('user_disable', { username: u.username, disabled: u.disabled ? 0 : 1 }), u.disabled ? 'User enabled' : 'User disabled and signed out'),
        }),
        el('button', {
          type: 'button',
          text: u.downloads ? 'Turn off downloads' : 'Allow downloads',
          onclick: () => run(
            () => admin('user_downloads', { username: u.username, enabled: u.downloads ? 0 : 1 }),
            u.downloads ? `Downloads turned off for ${u.username}` : `Downloads allowed for ${u.username}`,
          ),
        }),
        el('button', {
          type: 'button',
          text: 'New password',
          onclick: () => {
            const input = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'new password' });
            const save = el('button', {
              type: 'button',
              text: 'Save',
              onclick: () => {
                if (!validPassword(input.value)) return say('Passwords need at least 10 characters.', true);
                run(() => admin('user_password', { username: u.username, password: input.value }), 'Password changed');
              },
            });
            actions.replaceChildren(input, save);
            input.focus();
          },
        }),
        el('button', {
          type: 'button',
          text: 'Add a device',
          onclick: async () => {
            try {
              await showPairing(u.username);
            } catch (e) {
              say(e.message, true);
            }
          },
        }),
        ...(u.totp_enabled
          ? [confirmButton('Reset two-step', 'Confirm reset', () => run(() => admin('totp_reset', { username: u.username }), 'Two-step verification removed'))]
          : []),
        confirmButton('Delete', 'Confirm delete', () => run(() => admin('user_delete', { username: u.username }), 'User deleted')),
      ]);
      table.append(el('tr', {}, [
        el('td', { text: u.username }),
        el('td', { text: u.role }),
        el('td', { text: u.totp_enabled ? 'on' : 'off' }),
        el('td', { text: u.disabled ? 'disabled' : 'active' }),
        el('td', { text: downloadsText(u) }),
        el('td', {}, [grants]),
        el('td', {}, [actions]),
      ]));
      if (u.role === 'admin') {
        grants.textContent = 'all players';
      } else {
        showGrants(u.username, grants, players);
      }
    }
    box.replaceChildren(table);
  }

  // --- adding a device: a one-time pairing code, as text and as a QR code ------------------

  const SVG_NS = 'http://www.w3.org/2000/svg';
  let pairTimer = null;

  function closePairing() {
    if (pairTimer) clearInterval(pairTimer);
    pairTimer = null;
    const box = $('aw-pair');
    box.replaceChildren();
    box.setAttribute('hidden', '');
  }

  /** The QR code for a pairing link, drawn as one SVG path (black on white, quiet zone). */
  function qrImage(link) {
    const qr = globalThis.AnywhereQR.encodeQr(link);
    const n = qr.size + 8;
    const svg = document.createElementNS(SVG_NS, 'svg');
    for (const [k, v] of [['viewBox', `0 0 ${n} ${n}`], ['width', '240'], ['height', '240'], ['role', 'img'], ['aria-label', 'QR code for the Lyrion Anywhere app'], ['shape-rendering', 'crispEdges']]) {
      svg.setAttribute(k, v);
    }
    const bg = document.createElementNS(SVG_NS, 'rect');
    for (const [k, v] of [['width', String(n)], ['height', String(n)], ['fill', '#fff']]) bg.setAttribute(k, v);
    const path = document.createElementNS(SVG_NS, 'path');
    path.setAttribute('d', globalThis.AnywhereQR.qrPath(qr, 4));
    path.setAttribute('fill', '#000');
    svg.append(bg, path);
    return svg;
  }

  /**
   * Make a code for `username` and show it: large, as a QR code holding the pairing link
   * (when the public URL is set), and with the time it has left. The code is shown only
   * here: it isn't put in the status line, and nothing logs it.
   */
  async function showPairing(username) {
    closePairing();
    const d = await admin('pairing', { username });
    const view = pairingView(d, $('anywhere-app').getAttribute('data-public-url') || '', globalThis.AnywhereQR);
    if (!view) throw new Error('The gateway did not return a pairing code.');
    const qrBox = view.link
      ? el('div', { className: 'aw-qr' }, [qrImage(view.link)])
      : el('p', {
          className: 'aw-note aw-error',
          text: view.publicUrlSet
            ? 'No QR code: the Public URL above must be an https:// address with nothing after the host name. Until it is, enter the server address in the app, then type this code.'
            : 'No QR code: set the Public URL above (the https:// address people use) to show one. Until then, enter the server address in the app, then type this code.',
        });
    const code = el('p', { className: 'aw-code', text: view.code });
    const countdown = el('p', { className: 'aw-note' });
    const ends = Date.now() + view.seconds * 1000;
    const tick = () => {
      const left = Math.round((ends - Date.now()) / 1000);
      if (left > 0) {
        countdown.textContent = `Expires in ${formatCountdown(left)}. It works once, for ${username} only.`;
        return;
      }
      clearInterval(pairTimer);
      pairTimer = null;
      qrBox.setAttribute('hidden', '');
      code.className = 'aw-code aw-expired';
      countdown.textContent = 'This code has expired. Make a new one if you still need it.';
    };
    const box = $('aw-pair');
    box.replaceChildren(
      el('h4', { text: `Add a device for ${username}` }),
      el('p', { text: 'In the Lyrion Anywhere app on the new device, choose Scan QR code and point the camera at this code, or type the code by hand.' }),
      qrBox,
      code,
      countdown,
      el('button', { type: 'button', text: 'Done', onclick: closePairing }),
    );
    box.removeAttribute('hidden');
    tick();
    pairTimer = setInterval(tick, 1000);
  }

  async function showGrants(username, box, players) {
    try {
      const { players: granted = [] } = await admin('grants', { username });
      const list = granted.map((mac) =>
        el('div', { className: 'aw-grant' }, [
          el('span', { text: players.get(mac) || mac }),
          el('button', { type: 'button', text: 'Remove', onclick: () => run(() => admin('ungrant', { username, player: mac }), 'Player removed') }),
        ]),
      );
      const choices = [...players.entries()].filter(([mac]) => !granted.includes(mac) && !mac.startsWith('02:4c:41:'));
      const select = el('select', {}, [el('option', { value: '', text: 'Add a player' + ELLIPSIS }), ...choices.map(([mac, name]) => el('option', { value: mac, text: name }))]);
      select.addEventListener('change', () => {
        if (select.value) run(() => admin('grant', { username, player: select.value }), 'Player granted');
      });
      box.replaceChildren(...list, select);
    } catch (e) {
      box.textContent = e.message;
    }
  }

  async function showDevices() {
    const box = $('aw-devices');
    const { devices = [] } = await admin('devices');
    if (!devices.length) {
      box.replaceChildren(el('p', { className: 'aw-note', text: 'No devices signed in.' }));
      return;
    }
    const table = el('table', { className: 'aw-table' }, [
      el('tr', {}, ['Device', 'User', 'Client', 'Address', 'Last seen', 'Player id', ''].map((h) => el('th', { text: h }))),
    ]);
    for (const d of devices) {
      const client = el('td', {}, [el('div', { text: clientText(d) })]);
      if (typeof d.user_agent === 'string' && d.user_agent) {
        client.append(el('div', { className: 'aw-note aw-ua', text: d.user_agent }));
      }
      const name = el('td', {}, [el('span', { text: String(d.name) })]);
      table.append(el('tr', {}, [
        name,
        el('td', { text: d.username }),
        client,
        el('td', { text: typeof d.last_addr === 'string' && d.last_addr ? d.last_addr : DASH, className: 'aw-mono' }),
        el('td', { text: formatTime(d.last_seen) }),
        el('td', { text: d.mac, className: 'aw-mono' }),
        el('td', {}, [
          renameButton(d, name),
          diagnosticsButton(d),
          confirmButton('Sign out', 'Confirm sign-out', () => run(() => admin('revoke', { id: d.id }), 'Device signed out')),
        ]),
      ]));
    }
    box.replaceChildren(table);
  }

  /** Rename a device: an input in place of its name. Its LMS player follows. */
  function renameButton(d, cell) {
    return el('button', {
      type: 'button',
      text: 'Rename',
      onclick: () => {
        const input = el('input', { type: 'text', maxlength: '64', spellcheck: 'false', 'aria-label': 'New name' });
        input.value = String(d.name);
        const save = el('button', {
          type: 'button',
          text: 'Save',
          onclick: () => {
            const name = validDeviceName(input.value);
            if (name === null) return say('Device names are 1 to 64 characters.', true);
            run(() => admin('device_rename', { id: d.id, name }), `Device renamed to ${name}`);
          },
        });
        cell.replaceChildren(input, save);
        input.focus();
      },
    });
  }

  /** Diagnostics the device reported (playback mode, browser capabilities, recent log). */
  function diagnosticsButton(d) {
    const out = el('pre', { className: 'aw-diag', hidden: '' });
    const b = el('button', {
      type: 'button',
      text: 'Diagnostics',
      onclick: async () => {
        if (!out.hasAttribute('hidden')) {
          out.setAttribute('hidden', '');
          return;
        }
        try {
          const r = await admin('diagnostics', { id: d.id });
          out.textContent = diagnosticsText(r);
        } catch (e) {
          out.textContent = e.message;
        }
        out.removeAttribute('hidden');
      },
    });
    return el('div', { className: 'aw-diag-wrap' }, [b, out]);
  }

  async function showAudit() {
    const box = $('aw-audit');
    const { audit = [] } = await admin('audit', { limit: 50 });
    const table = el('table', { className: 'aw-table' }, [
      el('tr', {}, ['When', 'Event', 'User', 'Detail'].map((h) => el('th', { text: h }))),
      ...audit.map((a) =>
        el('tr', {}, [
          el('td', { text: formatTime(a.at) }),
          el('td', { text: a.event }),
          el('td', { text: a.username || '' }),
          el('td', { text: a.detail || '' }),
        ]),
      ),
    ]);
    box.replaceChildren(table);
  }

  async function refresh() {
    const status = await showStatus().catch((e) => say(e.message, true));
    if (!status || !status.running) {
      for (const id of ['aw-users', 'aw-devices', 'aw-audit']) {
        $(id).replaceChildren(el('p', { className: 'aw-note', text: 'Start the gateway to manage users and devices.' }));
      }
      return;
    }
    try {
      const players = await playerNames();
      await Promise.all([showUsers(players), showDevices(), showAudit(), showSettings(), showPlugins()]);
    } catch (e) {
      say(e.message, true);
    }
  }

  async function showSettings() {
    const s = await admin('settings');
    const input = $('aw-session-days');
    if (document.activeElement !== input) input.value = String(sessionDaysShown(s));
  }

  const KIND = { app: 'App', radio: 'Radio', menu: 'Menu', 'not installed': 'Not installed' };
  let pluginBoxes = [];

  async function showPlugins() {
    let r;
    try {
      r = await admin('plugins');
    } catch (e) {
      // Never leave the section blank: say why, and offer to try again (e.g. while the
      // gateway is still starting after LMS restarted).
      const retry = el('button', { type: 'button', text: 'Try again' });
      retry.addEventListener('click', () => {
        $('aw-plugins').replaceChildren(el('p', { className: 'aw-note', text: 'Loading plugins\u2026' }));
        showPlugins();
      });
      $('aw-plugins').replaceChildren(
        el('p', { className: 'aw-note aw-error', text: `Couldn't load the plugin list: ${e.message}` }),
        retry,
      );
      return;
    }
    const list = Array.isArray(r.plugins) ? r.plugins : [];
    pluginBoxes = [];
    const rows = list.map((p) => {
      const box = el('input', { type: 'checkbox' });
      box.checked = p.admin_only === true;
      const commands = Array.isArray(p.commands) ? p.commands.map(String) : [String(p.command)];
      pluginBoxes.push({ box, commands });
      return el('label', { className: 'aw-plugin' }, [
        box,
        el('span', { text: ` ${String(p.name)} ` }),
        el('span', { className: 'aw-note', text: `(${KIND[p.kind] || 'Plugin'}${DOT}${commands.join(', ')})` }),
      ]);
    });
    $('aw-plugins').replaceChildren(...(rows.length ? rows : [el('p', { className: 'aw-note', text: 'No plugins found (the gateway asks LMS; a player must be connected for My Music plugins).' })]));
  }

  function wirePlugins() {
    $('aw-plugins-btn').addEventListener('click', () => {
      const commands = adminOnlyArg(pluginBoxes.map((p) => ({ commands: p.commands, checked: p.box.checked })));
      const n = commands ? commands.split(',').length : 0;
      run(() => admin('plugins_admin_only', { commands }), n ? `${n} plugin${n === 1 ? ' is' : 's are'} now for admins only` : 'All plugins are open to listeners');
    });
  }

  function wireSettings() {
    $('aw-session-btn').addEventListener('click', () => {
      const days = validDays($('aw-session-days').value);
      if (days === null) return say('Enter a number of days from 1 to 365.', true);
      run(() => admin('session_days', { days }), `Devices now stay signed in for ${days} day${days === 1 ? '' : 's'} without use`);
    });
  }

  function wireCreate() {
    $('aw-create-btn').addEventListener('click', () => {
      const username = $('aw-new-user').value.trim();
      const password = $('aw-new-password').value;
      const role = $('aw-new-role').value;
      if (!validUsername(username)) return say('Usernames are 1 to 64 characters: a to z, 0 to 9, dot, underscore or hyphen.', true);
      if (!validPassword(password)) return say('Passwords need at least 10 characters.', true);
      run(async () => {
        await admin('user_create', { username, password, role });
        $('aw-new-user').value = '';
        $('aw-new-password').value = '';
      }, `User ${username} added`);
    });
  }

  document.addEventListener('DOMContentLoaded', () => {
    csrf = pageToken(document);
    wireCreate();
    wireSettings();
    wirePlugins();
    refresh();
    setInterval(() => showStatus().catch(() => {}), 10000);
  });
})();
