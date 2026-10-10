// Lyrion Anywhere settings page: users, devices and activity, via `anywhere …` commands.
//
// Names here (usernames, device names, audit details) can come from remote users, so the
// page is built with textContent only: never innerHTML or similar.
(function () {
  'use strict';

  // --- pure helpers (also used by the tests) ------------------------------------------

  /** The JSON-RPC command for an admin operation: ["anywhere", op, "key:value", …]. */
  function command(op, args) {
    const out = ['anywhere', op];
    for (const [k, v] of Object.entries(args || {})) {
      if (v === undefined || v === null || v === '') continue;
      out.push(k + ':' + String(v));
    }
    return out;
  }

  function validUsername(u) {
    return /^[a-z0-9._-]{1,64}$/.test(u);
  }

  function validPassword(p) {
    return typeof p === 'string' && p.length >= 10 && p.length <= 1024 && !/[\u0000-\u001f\u007f]/.test(p);
  }

  function formatTime(unixSeconds) {
    if (!unixSeconds) return '–';
    return new Date(unixSeconds * 1000).toLocaleString('en-GB');
  }

  function formatUptime(s) {
    if (!s) return '–';
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

  const api = { command, validUsername, validPassword, formatTime, formatUptime };
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

  async function admin(op, args) {
    const r = await rpc(command(op, args));
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
      ['Listening on', s.listen || '–'],
      ['Binary', s.binary_ok ? s.binary : `missing: ${s.binary || 'none for this platform'}`],
      ['Data', s.data_dir || '–'],
    ];
    if (s.error) rows.push(['Last error', s.error]);
    const box = $('aw-status');
    box.replaceChildren(
      el('dl', {}, rows.flatMap(([k, v]) => [el('dt', { text: k }), el('dd', { text: String(v) })])),
      el('button', { type: 'button', text: 'Restart gateway', onclick: () => run(() => rpc(['anywhere', 'restart']), 'Gateway restarted') }),
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
      el('tr', {}, ['User', 'Role', 'Two-step', 'Status', 'Home players', ''].map((h) => el('th', { text: h }))),
    ]);
    for (const u of users) {
      const grants = el('div', { className: 'aw-grants', text: '…' });
      const actions = el('div', { className: 'aw-actions' }, [
        el('button', {
          type: 'button',
          text: u.disabled ? 'Enable' : 'Disable',
          onclick: () => run(() => admin('user_disable', { username: u.username, disabled: u.disabled ? 0 : 1 }), u.disabled ? 'User enabled' : 'User disabled and signed out'),
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
          text: 'Pairing code',
          onclick: () => run(async () => {
            const d = await admin('pairing', { username: u.username });
            say(`Pairing code for ${u.username}: ${d.code} (valid ${Math.round(d.expires_in / 60)} minutes, single use)`);
          }),
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
      const select = el('select', {}, [el('option', { value: '', text: 'Add a player…' }), ...choices.map(([mac, name]) => el('option', { value: mac, text: name }))]);
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
      el('tr', {}, ['Device', 'User', 'Player id', 'Last seen', ''].map((h) => el('th', { text: h }))),
    ]);
    for (const d of devices) {
      table.append(el('tr', {}, [
        el('td', { text: d.name }),
        el('td', { text: d.username }),
        el('td', { text: d.mac, className: 'aw-mono' }),
        el('td', { text: formatTime(d.last_seen) }),
        el('td', {}, [confirmButton('Sign out', 'Confirm sign-out', () => run(() => admin('revoke', { id: d.id }), 'Device signed out'))]),
      ]));
    }
    box.replaceChildren(table);
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
      await Promise.all([showUsers(players), showDevices(), showAudit()]);
    } catch (e) {
      say(e.message, true);
    }
  }

  function wireCreate() {
    $('aw-create-btn').addEventListener('click', () => {
      const username = $('aw-new-user').value.trim();
      const password = $('aw-new-password').value;
      const role = $('aw-new-role').value;
      if (!validUsername(username)) return say('Usernames are 1–64 characters: a–z, 0–9, dot, underscore or hyphen.', true);
      if (!validPassword(password)) return say('Passwords need at least 10 characters.', true);
      run(async () => {
        await admin('user_create', { username, password, role });
        $('aw-new-user').value = '';
        $('aw-new-password').value = '';
      }, `User ${username} added`);
    });
  }

  document.addEventListener('DOMContentLoaded', () => {
    wireCreate();
    refresh();
    setInterval(() => showStatus().catch(() => {}), 10000);
  });
})();
