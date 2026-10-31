// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Which server a window talks to. `local` runs the bundled `vellum serve`
 * on this machine; `remote` connects to a team server.
 */
export type Settings =
  | { readonly mode: "local" }
  | { readonly mode: "remote"; readonly serverUrl: string };

/** Reads and writes one small file in the app's data folder. */
export interface Disk {
  read(): Promise<Uint8Array | undefined>;
  write(bytes: Uint8Array): Promise<void>;
  remove(): Promise<void>;
}

export const DEFAULT_SETTINGS: Settings = { mode: "local" };

const loopbackHosts = new Set(["127.0.0.1", "[::1]", "localhost"]);

/**
 * Normalises a server address to its origin. Plain HTTP is accepted only
 * on loopback, so tokens never cross a network in the clear.
 */
export function normalizeServerUrl(input: string): string {
  const url = new URL(input.includes("://") ? input : `https://${input}`);
  if (
    url.protocol !== "https:" &&
    !(url.protocol === "http:" && loopbackHosts.has(url.hostname))
  ) {
    throw new Error(`Use an https:// address for ${url.host}`);
  }
  if (url.username !== "" || url.password !== "") {
    throw new Error(
      "A server address must not contain a user name or password",
    );
  }
  return url.origin;
}

/**
 * Loads settings, falling back to local mode when the file is missing or
 * unreadable.
 */
export async function loadSettings(disk: Disk): Promise<Settings> {
  const bytes = await disk.read();
  if (bytes === undefined) return DEFAULT_SETTINGS;
  try {
    const value: unknown = JSON.parse(new TextDecoder().decode(bytes));
    if (isRemote(value))
      return { mode: "remote", serverUrl: normalizeServerUrl(value.serverUrl) };
  } catch {
    // A damaged file falls back to local mode rather than blocking start-up.
  }
  return DEFAULT_SETTINGS;
}

export async function saveSettings(
  disk: Disk,
  settings: Settings,
): Promise<void> {
  await disk.write(
    new TextEncoder().encode(`${JSON.stringify(settings, undefined, 2)}\n`),
  );
}

/** Reads `--server=<url>` or `--local` from the command line. */
export function settingsFromArgv(
  argv: readonly string[],
): Settings | undefined {
  if (argv.includes("--local")) return { mode: "local" };
  const flag = argv.find((argument) => argument.startsWith("--server="));
  return flag === undefined
    ? undefined
    : {
        mode: "remote",
        serverUrl: normalizeServerUrl(flag.slice("--server=".length)),
      };
}

function isRemote(
  value: unknown,
): value is { mode: "remote"; serverUrl: string } {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return record["mode"] === "remote" && typeof record["serverUrl"] === "string";
}
