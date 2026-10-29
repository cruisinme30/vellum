// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  allowPermission,
  contentSecurityPolicy,
  isAppUrl,
  isSafeExternal,
  webPreferences,
} from "./security.js";

describe("security", () => {
  it("recognises app pages only on app://vellum", () => {
    expect(isAppUrl("app://vellum/p/1")).toBe(true);
    expect(isAppUrl("app://other/")).toBe(false);
    expect(isAppUrl("https://vellum/")).toBe(false);
    expect(isAppUrl("not a url")).toBe(false);
  });

  it("opens only web and mail links outside the app", () => {
    expect(isSafeExternal("https://northstar.dev/a")).toBe(true);
    expect(isSafeExternal("http://example.com")).toBe(true);
    expect(isSafeExternal("mailto:ana@northstar.dev")).toBe(true);
    expect(isSafeExternal("file:///etc/passwd")).toBe(false);
    expect(isSafeExternal("javascript:alert(1)")).toBe(false);
    expect(isSafeExternal("https://user:pw@example.com")).toBe(false);
    expect(isSafeExternal("::")).toBe(false);
  });

  it("grants a short list of permissions to app pages", () => {
    expect(allowPermission("notifications", "app://vellum/")).toBe(true);
    expect(allowPermission("notifications", "https://evil.example/")).toBe(false);
    expect(allowPermission("media", "app://vellum/")).toBe(false);
  });

  it("limits direct connections to the server's WebSocket", () => {
    expect(contentSecurityPolicy("https://vellum.northstar.dev")).toContain(
      "connect-src 'self' wss://vellum.northstar.dev",
    );
    expect(contentSecurityPolicy("http://127.0.0.1:7700")).toContain(
      "connect-src 'self' ws://127.0.0.1:7700",
    );
    expect(contentSecurityPolicy("https://a.dev")).toContain("object-src 'none'");
  });

  it("sandboxes the renderer and passes the version to the preload", () => {
    const preferences = webPreferences("/p.cjs", "1.2.3");
    expect(preferences).toMatchObject({
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
    });
    expect(preferences.additionalArguments).toEqual(["--vellum-version=1.2.3"]);
  });
});
