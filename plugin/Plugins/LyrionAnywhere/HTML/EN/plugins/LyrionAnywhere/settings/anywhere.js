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

  /** The gateway's audit event codes, in words. */
  const AUDIT_EVENTS = {
    'login.ok': 'Signed in',
    'login.failed': 'Sign-in failed',
    'login.totp_failed': 'Sign-in failed (wrong two-step code)',
    logout: 'Signed out',
    'pairing.created': 'Device code made',
    'pairing.ok': 'Device added with a code',
    'pairing.failed': 'Device code refused',
    'pairing.rate_limited': 'Too many device codes asked for',
    'pairing.self_failed': 'Device code refused (wrong password)',
    'refresh.reused': 'Old sign-in reused: device signed out',
    'refresh.retried': 'Sign-in renewed again',
    'device.revoked': 'Device signed out',
    'device.renamed': 'Device renamed',
    'device.evicted': 'Oldest device signed out (device limit reached)',
    'device.rate_limited': 'Too many new devices',
    'user.created': 'User added',
    'user.deleted': 'User deleted',
    'user.disabled': 'User disabled',
    'user.enabled': 'User enabled',
    'user.password_changed': 'Password changed',
    'user.downloads': 'Downloads changed',
    'totp.enabled': 'Two-step turned on',
    'totp.disabled': 'Two-step turned off',
    'totp.reset': 'Two-step removed by an admin',
    'totp.setup_failed': 'Two-step set-up failed',
    'totp.disable_failed': 'Two-step not turned off (wrong password)',
    'player.granted': 'Player given',
    'player.ungranted': 'Player taken away',
    'settings.session_days': 'Sign-in lifetime changed',
    'settings.admin_only_plugins': 'Admin-only plugins changed',
    'history.cleared': 'Play history cleared',
    download: 'Track downloaded',
    plays: 'Offline plays recorded',
  };

  /** An audit event in words; an unknown code is shown as it is. */
  function auditEventText(code) {
    const c = String(code ?? '');
    return Object.prototype.hasOwnProperty.call(AUDIT_EVENTS, c) ? AUDIT_EVENTS[c] : c;
  }

  /** Is a public URL a plain http:// one (sign-ins would cross the internet unencrypted)? */
  function isHttpUrl(v) {
    return /^\s*http:\/\//i.test(String(v ?? ''));
  }

  /** The Enter key (not while an input method is composing text). */
  function isEnter(e) {
    return !!e && e.key === 'Enter' && !e.isComposing && e.keyCode !== 229;
  }

  /** What the page says after "Restart gateway", from the command's result. */
  function restartText(r) {
    if (r && (r.started === 0 || r.started === '0')) {
      return typeof r.message === 'string' && r.message ? r.message : 'Remote access is turned off, so the gateway was not started.';
    }
    return 'Gateway restarted';
  }

  /** What to say, once, when the gateway can't be used (null while it is running). */
  function offlineText(s) {
    if (s && s.running) return null;
    if (s && !s.enabled) return 'Remote access is turned off. Tick Enabled above and save the settings to manage users, devices and sign-in.';
    return 'The gateway is not running, so users, devices and sign-in can\u2019t be managed until it starts. See Last error above, or use Restart gateway.';
  }

  const api = { command, adminOnlyArg, pageToken, validDays, sessionDaysShown, validDeviceName, clientText, validUsername, validPassword, formatTime, formatUptime, diagnosticsText, downloadsText, formatCountdown, pairingView, auditEventText, isHttpUrl, isEnter, restartText, offlineText, DEFAULT_SESSION_DAYS };
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

  // --- messages: one live region, shown next to the section last acted on ----------------

  let actionPanel = null; // the panel (.aw-panel) holding the control last used

  function say(text, isError) {
    const m = $('aw-message');
    const app = $('anywhere-app');
    m.textContent = '';
    if (actionPanel && actionPanel.isConnected && app.contains(actionPanel)) actionPanel.after(m);
    else app.prepend(m);
    m.className = 'aw-message' + (isError ? ' aw-error' : '');
    m.textContent = text;
  }

  // --- focus: rebuilt lists keep it on the same control, else on the section heading ------

  const PANEL_HEADING = {
    'aw-status': 'aw-h-status',
    'aw-users': 'aw-h-users',
    'aw-pair': 'aw-h-users',
    'aw-create': 'aw-h-users',
    'aw-session': 'aw-h-signin',
    'aw-plugins-box': 'aw-h-plugins',
    'aw-devices': 'aw-h-devices',
    'aw-audit': 'aw-h-audit',
  };

  function focusPlace() {
    const a = document.activeElement;
    if (!a || !$('anywhere-app').contains(a)) return null;
    const panel = a.closest('.aw-panel');
    return { key: a.getAttribute('data-aw-key') || '', heading: panel ? PANEL_HEADING[panel.id] : null };
  }

  /** Put focus back after a rebuild, unless it is still somewhere sensible. */
  function restoreFocus(place) {
    if (!place) return;
    const a = document.activeElement;
    if (a && a !== document.body && a.isConnected) return;
    let target = null;
    if (place.key) {
      for (const e of document.querySelectorAll('[data-aw-key]')) {
        if (e.getAttribute('data-aw-key') === place.key && !e.disabled) target = e;
      }
    }
    target = target || (place.heading && $(place.heading));
    if (target) target.focus();
  }

  // Enter in one of this section's fields runs that field's action; it must never submit
  // LMS's settings form around the page (which would reload it and lose what was typed).
  const enterButtons = new WeakMap();

  function onEnter(input, button) {
    enterButtons.set(input, button);
    return input;
  }

  /**
   * A button that needs a second click to confirm (no browser dialogs). `subject` names what
   * it acts on, for its accessible name (e.g. "Delete: alice").
   */
  function confirmButton(label, confirmLabel, action, subject, key) {
    const name = (t) => (subject ? `${t}: ${subject}` : t);
    const b = el('button', { type: 'button', text: label, 'aria-label': name(label) });
    if (key) b.setAttribute('data-aw-key', key);
    let armed = false;
    let timer = null;
    b.addEventListener('click', async () => {
      if (!armed) {
        armed = true;
        b.textContent = confirmLabel;
        b.setAttribute('aria-label', name(confirmLabel));
        b.classList.add('aw-danger');
        timer = setTimeout(() => {
          armed = false;
          b.textContent = label;
          b.setAttribute('aria-label', name(label));
          b.classList.remove('aw-danger');
        }, 4000);
        return;
      }
      clearTimeout(timer);
      armed = false;
      b.textContent = label;
      b.setAttribute('aria-label', name(label));
      b.classList.remove('aw-danger');
      await action();
    });
    return b;
  }

  async function run(action, done) {
    const place = focusPlace();
    try {
      await action();
      if (done) say(done);
      await refresh();
    } catch (e) {
      say(e.message, true);
    }
    restoreFocus(place);
  }

  // --- views ------------------------------------------------------------------------------

  let live = null; // is the gateway running (as last shown)?

  /** Turn the controls that need the gateway on or off, and say once why they are off. */
  function setLive(s) {
    const text = offlineText(s);
    live = text === null;
    for (const id of ['aw-create', 'aw-session', 'aw-plugins-box']) $(id).disabled = !live;
    const note = $('aw-offline');
    note.textContent = text || '';
    if (text) note.removeAttribute('hidden');
    else note.setAttribute('hidden', '');
    if (!live) {
      closePairing();
      for (const id of ['aw-users', 'aw-devices', 'aw-audit']) $(id).replaceChildren();
      $('aw-plugins').replaceChildren();
    }
  }

  let restartButton = null;
  let restartNote = null;
  let statusInfo = null;

  async function showStatus() {
    const s = await rpc(['anywhere', 'status']);
    const rows = [
      ['Gateway', s.running ? `running (pid ${s.pid})` : s.enabled ? 'not running' : 'turned off'],
      ['Up for', formatUptime(s.uptime)],
      ['Listening on', s.listen || DASH],
      ['Binary', s.binary_ok ? s.binary : `missing: ${s.binary || 'none for this platform'}`],
      ['Data', s.data_dir || DASH],
    ];
    if (s.error) rows.push(['Last error', s.error]);
    // Gateway settings someone changed with LMS's `pref` command, which were put back.
    if (s.settings_notice) rows.push(['Settings', s.settings_notice]);
    const box = $('aw-status');
    // The button is made once, so the 10-second status update never takes focus away from it
    // or forgets that it is waiting for a confirming click.
    if (!restartButton) {
      restartButton = confirmButton('Restart gateway', 'Confirm restart', () => run(async () => {
        const r = await rpc(command('restart', {}, csrf));
        if (r.error) throw new Error(r.error);
        say(restartText(r));
      }), null, 'restart');
      statusInfo = el('div', {});
      restartNote = el('p', { className: 'aw-note', id: 'aw-restart-note' });
      restartButton.setAttribute('aria-describedby', 'aw-restart-note');
      box.replaceChildren(statusInfo, restartNote, restartButton);
    }
    restartButton.disabled = !s.enabled;
    restartNote.textContent = s.enabled
      ? 'Restarting disconnects everyone listening remotely for a few seconds.'
      : 'Remote access is turned off: tick Enabled above and save the settings to start the gateway.';
    statusInfo.replaceChildren(
      el('dl', {}, rows.flatMap(([k, v]) => [el('dt', { text: k }), el('dd', { text: String(v) })])),
    );
    return s;
  }

  async function playerNames() {
    const r = await rpc(['players', 0, 200]);
    const names = new Map();
    for (const p of r.players_loop || []) names.set(p.playerid, p.name);
    return names;
  }

  /** A button acting on one item: its accessible name says which, its key finds it again. */
  function itemButton(text, label, key, onclick) {
    return el('button', { type: 'button', text, 'aria-label': label, 'data-aw-key': key, onclick });
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
    const pending = [];
    for (const u of users) {
      const name = u.username;
      const grants = el('div', { className: 'aw-grants', text: ELLIPSIS });
      const actions = el('div', { className: 'aw-actions' }, [
        itemButton(
          u.disabled ? 'Enable' : 'Disable',
          `${u.disabled ? 'Enable' : 'Disable'} ${name}`,
          `user:${name}:disable`,
          () => run(() => admin('user_disable', { username: name, disabled: u.disabled ? 0 : 1 }), u.disabled ? 'User enabled' : 'User disabled and signed out'),
        ),
        itemButton(
          u.downloads ? 'Turn off downloads' : 'Allow downloads',
          `${u.downloads ? 'Turn off downloads for' : 'Allow downloads for'} ${name}`,
          `user:${name}:downloads`,
          () => run(
            () => admin('user_downloads', { username: name, enabled: u.downloads ? 0 : 1 }),
            u.downloads ? `Downloads turned off for ${name}` : `Downloads allowed for ${name}`,
          ),
        ),
        itemButton('New password', `New password for ${name}`, `user:${name}:password`, () => {
          const input = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'new password', 'aria-label': `New password for ${name}` });
          const save = itemButton('Save', `Save the new password for ${name}`, `user:${name}:password-save`, () => {
            if (!validPassword(input.value)) return say('Passwords need at least 10 characters.', true);
            run(() => admin('user_password', { username: name, password: input.value }), 'Password changed');
          });
          actions.replaceChildren(onEnter(input, save), save);
          input.focus();
        }),
        itemButton('Add a device', `Add a device for ${name}`, `user:${name}:pair`, async () => {
          try {
            await showPairing(name);
          } catch (e) {
            say(e.message, true);
          }
        }),
        ...(u.totp_enabled
          ? [confirmButton('Reset two-step', 'Confirm reset', () => run(() => admin('totp_reset', { username: name }), 'Two-step verification removed'), name, `user:${name}:totp`)]
          : []),
        confirmButton('Delete', 'Confirm delete', () => run(() => admin('user_delete', { username: name }), 'User deleted'), name, `user:${name}:delete`),
      ]);
      table.append(el('tr', {}, [
        el('td', { text: name }),
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
        pending.push(showGrants(name, grants, players));
      }
    }
    box.replaceChildren(table);
    await Promise.all(pending);
  }

  // --- adding a device: a one-time pairing code, as text and as a QR code ------------------

  const SVG_NS = 'http://www.w3.org/2000/svg';
  let pairTimer = null;

  function closePairing() {
    if (pairTimer) clearInterval(pairTimer);
    pairTimer = null;
    const box = $('aw-pair');
    const hadFocus = box.contains(document.activeElement);
    box.replaceChildren();
    box.setAttribute('hidden', '');
    if (hadFocus) $('aw-h-users').focus();
  }

  /** The QR code for a pairing link, drawn as one SVG path (black on white, quiet zone). */
  function qrImage(link) {
    const qr = globalThis.AnywhereQR.encodeQr(link);
    const n = qr.size + 8;
    const svg = document.createElementNS(SVG_NS, 'svg');
    for (const [k, v] of [['viewBox', `0 0 ${n} ${n}`], ['width', '240'], ['height', '240'], ['role', 'img'], ['aria-label', 'QR code for the Lyrion Anywhere Android app'], ['shape-rendering', 'crispEdges']]) {
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
    const heading = el('h4', { text: `Add a device for ${username}`, tabindex: '-1' });
    const box = $('aw-pair');
    box.replaceChildren(
      heading,
      el('p', { text: 'In the Lyrion Anywhere Android app on the new device, choose Scan QR code and point the camera at this code, or type the code in.' }),
      el('p', { className: 'aw-note', text: `In a web browser, codes aren\u2019t used: open the public address and sign in with ${username}\u2019s username and password.` }),
      qrBox,
      code,
      countdown,
      el('button', { type: 'button', text: 'Done', 'aria-label': `Done adding a device for ${username}`, onclick: closePairing }),
    );
    box.removeAttribute('hidden');
    tick();
    pairTimer = setInterval(tick, 1000);
    heading.focus();
  }

  async function showGrants(username, box, players) {
    try {
      const { players: granted = [] } = await admin('grants', { username });
      const list = granted.map((mac) => {
        const name = players.get(mac) || mac;
        return el('div', { className: 'aw-grant' }, [
          el('span', { text: name }),
          itemButton('Remove', `Remove ${name} from ${username}`, `grant:${username}:${mac}`, () => run(() => admin('ungrant', { username, player: mac }), 'Player removed')),
        ]);
      });
      const choices = [...players.entries()].filter(([mac]) => !granted.includes(mac) && !mac.startsWith('02:4c:41:'));
      if (!choices.length) {
        box.replaceChildren(...list);
        return;
      }
      // Choosing a player does nothing by itself: the Add button gives it.
      const select = el('select', { 'aria-label': `Player to give ${username}`, 'data-aw-key': `grant:${username}:choose` }, [
        el('option', { value: '', text: 'Choose a player' + ELLIPSIS }),
        ...choices.map(([mac, name]) => el('option', { value: mac, text: name })),
      ]);
      const add = itemButton('Add', `Give ${username} the chosen player`, `grant:${username}:add`, () => {
        if (!select.value) return say(`Choose a player to give ${username} first.`, true);
        const name = players.get(select.value) || select.value;
        run(() => admin('grant', { username, player: select.value }), `${name} given to ${username}`);
      });
      box.replaceChildren(...list, el('div', { className: 'aw-grant' }, [select, add]));
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
          confirmButton('Sign out', 'Confirm sign-out', () => run(() => admin('revoke', { id: d.id }), 'Device signed out'), String(d.name), `device:${d.id}:revoke`),
        ]),
      ]));
    }
    box.replaceChildren(table);
  }

  /** Rename a device: an input in place of its name. Its LMS player follows. */
  function renameButton(d, cell) {
    return itemButton('Rename', `Rename ${d.name}`, `device:${d.id}:rename`, () => {
      const input = el('input', { type: 'text', maxlength: '64', spellcheck: 'false', 'aria-label': `New name for ${d.name}` });
      input.value = String(d.name);
      const save = itemButton('Save', `Save the new name for ${d.name}`, `device:${d.id}:rename-save`, () => {
        const name = validDeviceName(input.value);
        if (name === null) return say('Device names are 1 to 64 characters.', true);
        run(() => admin('device_rename', { id: d.id, name }), `Device renamed to ${name}`);
      });
      cell.replaceChildren(onEnter(input, save), save);
      input.focus();
    });
  }

  /** Diagnostics the device reported (playback mode, browser capabilities, recent log). */
  function diagnosticsButton(d) {
    const out = el('pre', { className: 'aw-diag', hidden: '', id: `aw-diag-${d.id}` });
    const b = itemButton('Diagnostics', `Diagnostics for ${d.name}`, `device:${d.id}:diag`, async () => {
      if (!out.hasAttribute('hidden')) {
        out.setAttribute('hidden', '');
        b.setAttribute('aria-expanded', 'false');
        return;
      }
      try {
        const r = await admin('diagnostics', { id: d.id });
        out.textContent = diagnosticsText(r);
      } catch (e) {
        out.textContent = e.message;
      }
      out.removeAttribute('hidden');
      b.setAttribute('aria-expanded', 'true');
    });
    b.setAttribute('aria-expanded', 'false');
    b.setAttribute('aria-controls', out.id);
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
          el('td', { text: auditEventText(a.event), title: String(a.event ?? '') }),
          el('td', { text: a.username || '' }),
          el('td', { text: a.detail || '' }),
        ]),
      ),
    ]);
    box.replaceChildren(table);
  }

  async function refresh() {
    const status = await showStatus().catch((e) => {
      say(e.message, true);
      return null;
    });
    setLive(status);
    if (!live) return;
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
      const retry = el('button', { type: 'button', text: 'Try again', 'aria-label': 'Try loading the plugin list again', 'data-aw-key': 'plugins:retry' });
      retry.addEventListener('click', () => {
        $('aw-plugins').replaceChildren(el('p', { className: 'aw-note', text: 'Loading plugins\u2026' }));
        showPlugins().then(() => restoreFocus({ key: '', heading: 'aw-h-plugins' }));
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
      const commands = Array.isArray(p.commands) ? p.commands.map(String) : [String(p.command)];
      const box = el('input', { type: 'checkbox', 'data-aw-key': `plugin:${commands.join(',')}` });
      box.checked = p.admin_only === true;
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
    onEnter($('aw-session-days'), $('aw-session-btn'));
    $('aw-session-btn').addEventListener('click', () => {
      const days = validDays($('aw-session-days').value);
      if (days === null) return say('Enter a number of days from 1 to 365.', true);
      run(() => admin('session_days', { days }), `Devices now stay signed in for ${days} day${days === 1 ? '' : 's'} without use`);
    });
  }

  function wireCreate() {
    onEnter($('aw-new-user'), $('aw-create-btn'));
    onEnter($('aw-new-password'), $('aw-create-btn'));
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

  function wirePage() {
    const app = $('anywhere-app');
    // Remember which section was used last, so messages appear beside it.
    const note = (e) => {
      const p = e.target && e.target.closest ? e.target.closest('.aw-panel') : null;
      if (p) actionPanel = p;
    };
    app.addEventListener('click', note, true);
    app.addEventListener('keydown', (e) => {
      if (!isEnter(e)) return;
      const t = e.target;
      if (!t || t.tagName !== 'INPUT') return;
      note(e);
      e.preventDefault();
      const b = enterButtons.get(t);
      if (b && !b.disabled) b.click();
    });
    // Warn at once about an http:// public URL (saving it also needs "Allow http://").
    const url = $('pref_public_url');
    const warning = $('aw-http-warning');
    if (url && warning) {
      const update = () => {
        if (isHttpUrl(url.value)) warning.removeAttribute('hidden');
        else warning.setAttribute('hidden', '');
      };
      url.addEventListener('input', update);
      update();
    }
  }

  document.addEventListener('DOMContentLoaded', () => {
    csrf = pageToken(document);
    wirePage();
    wireCreate();
    wireSettings();
    wirePlugins();
    refresh();
    // Keep the status current; when the gateway starts or stops, rebuild the sections.
    setInterval(async () => {
      const s = await showStatus().catch(() => null);
      if (s && (offlineText(s) === null) !== live) refresh();
    }, 10000);
  });
})();
