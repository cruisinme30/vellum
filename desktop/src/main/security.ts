// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The renderer's security rules (ADR 0023, D99): pages load only from the
 * privileged `app://vellum/` scheme, run sandboxed without Node, and leave
 * the app only through the system browser.
 */

export const APP_SCHEME = "app";
export const APP_HOST = "vellum";
export const APP_URL = `${APP_SCHEME}://${APP_HOST}/`;

/** Whether `url` is one of the app's own pages. */
export function isAppUrl(url: string): boolean {
  const parsed = parse(url);
  return parsed?.protocol === `${APP_SCHEME}:` && parsed.host === APP_HOST;
}

/** Whether `url` may be handed to the system browser or mail client. */
export function isSafeExternal(url: string): boolean {
  const parsed = parse(url);
  if (parsed === undefined) return false;
  if (parsed.protocol === "mailto:") return true;
  return (
    (parsed.protocol === "https:" || parsed.protocol === "http:") &&
    parsed.username === "" &&
    parsed.password === ""
  );
}

const allowedPermissions = new Set([
  "notifications",
  "clipboard-sanitized-write",
  "fullscreen",
]);

/** Grants a permission only to app pages, and only from a short list. */
export function allowPermission(
  permission: string,
  requestingUrl: string,
): boolean {
  return allowedPermissions.has(permission) && isAppUrl(requestingUrl);
}

/**
 * The Content Security Policy for app pages. `server` is the Vellum server
 * the window talks to; only its WebSocket endpoint is reachable directly
 * (live editing), everything else goes through the `app://` API proxy.
 */
export function contentSecurityPolicy(server: string): string {
  const origin = new URL(server);
  const socket =
    (origin.protocol === "https:" ? "wss" : "ws") + `://${origin.host}`;
  return [
    "default-src 'self'",
    "script-src 'self' 'wasm-unsafe-eval'",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    `connect-src 'self' ${socket}`,
    "worker-src 'self' blob:",
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join("; ");
}

/** The subset of Electron's `WebPreferences` the app sets. */
interface WindowPreferences {
  preload: string;
  sandbox: true;
  contextIsolation: true;
  nodeIntegration: false;
  nodeIntegrationInWorker: false;
  webSecurity: true;
  allowRunningInsecureContent: false;
  webviewTag: false;
  spellcheck: boolean;
  additionalArguments: string[];
}

/** `webPreferences` for every app window. */
export function webPreferences(
  preload: string,
  version: string,
): WindowPreferences {
  return {
    preload,
    sandbox: true,
    contextIsolation: true,
    nodeIntegration: false,
    nodeIntegrationInWorker: false,
    webSecurity: true,
    allowRunningInsecureContent: false,
    webviewTag: false,
    spellcheck: true,
    additionalArguments: [`--vellum-version=${version}`],
  };
}

function parse(url: string): URL | undefined {
  try {
    return new URL(url);
  } catch {
    return undefined;
  }
}
