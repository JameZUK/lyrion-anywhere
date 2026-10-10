// Gateway authentication for the browser (docs/PROTOCOL.md, spec §9).
//
// The refresh token lives only in an HttpOnly cookie the page can't read. The short-lived
// access token is kept in memory. Every auth request carries X-Anywhere-Client, which a
// cross-site page can't send without a CORS preflight the gateway never approves.

const CLIENT = { 'X-Anywhere-Client': '1' };
const DEVICE_KEY = 'anywhere.device';

export class AuthError extends Error {
  constructor(status, message, totpRequired = false) {
    super(message);
    this.status = status;
    /** The password was right; ask for a two-step code and sign in again. */
    this.totpRequired = totpRequired;
  }
}

/** The device id this browser used last time (not a secret), if any. */
export function rememberedDevice(storage = globalThis.localStorage) {
  try {
    const v = Number(storage.getItem(DEVICE_KEY));
    return Number.isSafeInteger(v) && v > 0 ? v : undefined;
  } catch {
    return undefined; // storage blocked (private mode etc.)
  }
}

function rememberDevice(id, storage = globalThis.localStorage) {
  try {
    storage.setItem(DEVICE_KEY, String(id));
  } catch {
    /* not essential */
  }
}

/** A readable default name for this browser's player, e.g. "Firefox on Linux". */
export function defaultDeviceName(ua = navigator.userAgent) {
  const browser = /Firefox\//.test(ua)
    ? 'Firefox'
    : /Edg\//.test(ua)
      ? 'Edge'
      : /Chrome\//.test(ua)
        ? 'Chrome'
        : /Safari\//.test(ua)
          ? 'Safari'
          : 'Browser';
  const os = /Android/.test(ua)
    ? 'Android'
    : /iPhone|iPad|iPod/.test(ua)
      ? 'iOS'
      : /Mac OS X/.test(ua)
        ? 'macOS'
        : /Windows/.test(ua)
          ? 'Windows'
          : /Linux/.test(ua)
            ? 'Linux'
            : 'unknown';
  return `${browser} on ${os}`;
}

async function call(path, init) {
  const resp = await fetch(path, { credentials: 'same-origin', cache: 'no-store', ...init });
  if (resp.status === 204) return null;
  const body = await resp.json().catch(() => ({}));
  if (!resp.ok) throw new AuthError(resp.status, body.error || `HTTP ${resp.status}`, body.totp_required === true);
  return body;
}

/** Log in; returns the session ({access_token, expires_in, user, device}). */
export async function login(username, password, totp) {
  const session = await call('/v1/auth/login', {
    method: 'POST',
    headers: { ...CLIENT, 'Content-Type': 'application/json' },
    body: JSON.stringify({
      username,
      password,
      ...(totp ? { totp } : {}),
      device_name: defaultDeviceName(),
      device_id: rememberedDevice(),
    }),
  });
  rememberDevice(session.device.id);
  return session;
}

/** Exchange the refresh cookie for a new access token (and rotated cookie). */
export async function refresh() {
  const session = await call('/v1/auth/refresh', { method: 'POST', headers: CLIENT });
  rememberDevice(session.device.id);
  return session;
}

export async function logout(accessToken) {
  await call('/v1/auth/logout', {
    method: 'POST',
    headers: { ...CLIENT, Authorization: `Bearer ${accessToken}` },
  });
}

const bearer = (token) => ({ ...CLIENT, Authorization: `Bearer ${token}` });

/** The signed-in user's settings: {username, role, totp_enabled}. */
export async function account(token) {
  return call('/v1/account', { headers: { Authorization: `Bearer ${token}` } });
}

/** Start two-step setup: {secret, uri, qr_svg}. */
export async function totpSetup(token) {
  return call('/v1/account/totp/setup', { method: 'POST', headers: bearer(token) });
}

export async function totpConfirm(token, code) {
  await call('/v1/account/totp/confirm', {
    method: 'POST',
    headers: { ...bearer(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ code }),
  });
}

export async function totpDisable(token, password, code) {
  await call('/v1/account/totp/disable', {
    method: 'POST',
    headers: { ...bearer(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ password, code }),
  });
}

/** Group a base32 key in fours for reading aloud or typing. */
export function groupKey(secret) {
  return secret.replace(/(.{4})(?=.)/g, '$1 ');
}
