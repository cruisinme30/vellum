// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Fetch } from "./fetch.js";

/**
 * Local mode: runs the bundled `vellum serve` on a loopback port with the
 * `single` profile and a data folder under the app's user data, then waits
 * for `/readyz` before the window loads.
 */

export interface Child {
  /** Resolves with the exit code once the process ends. */
  readonly exited: Promise<number | null>;
  kill(): void;
}

export interface SidecarDeps {
  readonly spawn: (
    command: string,
    commandArguments: readonly string[],
    environment: Readonly<Record<string, string>>,
  ) => Child;
  readonly freePort: () => Promise<number>;
  readonly fetch: Fetch;
  readonly sleep: (ms: number) => Promise<void>;
  readonly now: () => number;
}

export interface SidecarOptions {
  readonly binary: string;
  readonly dataDir: string;
  readonly timeoutMs?: number;
}

export interface Sidecar {
  readonly url: string;
  stop(): void;
}

const POLL_MS = 100;

export async function startSidecar(options: SidecarOptions, deps: SidecarDeps): Promise<Sidecar> {
  const port = await deps.freePort();
  const url = `http://127.0.0.1:${String(port)}`;
  const child = deps.spawn(options.binary, ["serve"], {
    VELLUM_BIND: `127.0.0.1:${String(port)}`,
    VELLUM_PUBLIC_URL: url,
    VELLUM_DATA_DIR: options.dataDir,
    VELLUM_PROFILE: "single",
  });
  let exitCode: number | null | undefined;
  void child.exited.then((code) => {
    exitCode = code;
  });
  const deadline = deps.now() + (options.timeoutMs ?? 30_000);
  while (deps.now() < deadline) {
    if (exitCode !== undefined)
      throw new Error(`The local Vellum server exited with code ${String(exitCode)}`);
    if (await ready(url, deps.fetch))
      return {
        url,
        stop: () => {
          child.kill();
        },
      };
    await deps.sleep(POLL_MS);
  }
  child.kill();
  throw new Error("The local Vellum server did not become ready in time");
}

async function ready(url: string, fetcher: Fetch): Promise<boolean> {
  try {
    const response = await fetcher(`${url}/readyz`);
    return response.ok;
  } catch {
    return false;
  }
}
