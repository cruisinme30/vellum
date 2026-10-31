// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** `vellum://` links: `vellum://p/abc?x=1` opens the app path `/p/abc?x=1`. */

export const DEEP_LINK_SCHEME = "vellum";

const safePath = /^\/[\w\-./~%]*$/;

/**
 * The in-app path a `vellum://` link points at, or undefined if it is not one.
 */
export function deepLinkPath(url: string): string | undefined {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return undefined;
  }
  if (parsed.protocol !== `${DEEP_LINK_SCHEME}:`) return undefined;
  const path =
    `/${parsed.host}${parsed.pathname === "/" ? "" : parsed.pathname}`.replace(
      /\/$/,
      "",
    ) || "/";
  if (!safePath.test(path) || path.split("/").includes("..")) return undefined;
  return `${path}${parsed.search}`;
}

/**
 * Finds the first `vellum://` link in a command line (Windows and Linux pass it
 * there).
 */
export function findDeepLink(argv: readonly string[]): string | undefined {
  for (const argument of argv) {
    const path = deepLinkPath(argument);
    if (path !== undefined) return path;
  }
  return undefined;
}
