// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { startSidecar, type Child, type SidecarDeps } from "./sidecar.js";

function deps(
  readyAfter: number,
  exitCode?: number,
): SidecarDeps & {
  child: Child;
  kill: () => void;
  env: () => Record<string, string>;
} {
  let clock = 0;
  let polls = 0;
  let environment: Record<string, string> = {};
  const kill = vi.fn<() => void>();
  const child: Child = {
    exited:
      exitCode === undefined
        ? new Promise<number | null>(() => {
            // Never exits.
          })
        : Promise.resolve(exitCode),
    kill,
  };
  return {
    child,
    kill,
    env: () => environment,
    spawn: (_command, _arguments, environment_) => {
      environment = { ...environment_ };
      return child;
    },
    freePort: () => Promise.resolve(41_000),
    fetch: (url) => {
      polls += 1;
      expect(url).toBe("http://127.0.0.1:41000/readyz");
      if (polls === 1)
        return Promise.reject(new TypeError("connection refused"));
      return Promise.resolve(
        new Response(undefined, { status: polls > readyAfter ? 200 : 503 }),
      );
    },
    sleep: (ms) => {
      clock += ms;
      return Promise.resolve();
    },
    now: () => clock,
  };
}

describe("startSidecar", () => {
  it("starts vellum serve on loopback and waits for readiness", async () => {
    const fake = deps(3);
    const sidecar = await startSidecar(
      { binary: "/bin/vellum", dataDir: "/data" },
      fake,
    );
    expect(sidecar.url).toBe("http://127.0.0.1:41000");
    expect(fake.env()).toEqual({
      VELLUM_BIND: "127.0.0.1:41000",
      VELLUM_PUBLIC_URL: "http://127.0.0.1:41000",
      VELLUM_DATA_DIR: "/data",
      VELLUM_PROFILE: "single",
    });
    sidecar.stop();
    expect(fake.kill).toHaveBeenCalled();
  });

  it("fails when the server exits or never becomes ready", async () => {
    await expect(
      startSidecar({ binary: "b", dataDir: "d" }, deps(1000, 2)),
    ).rejects.toThrow(/exited with code 2/);
    const slow = deps(1000);
    await expect(
      startSidecar({ binary: "b", dataDir: "d", timeoutMs: 500 }, slow),
    ).rejects.toThrow(/ready in time/);
    expect(slow.kill).toHaveBeenCalled();
  });
});
