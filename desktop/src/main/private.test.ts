// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { PrivateSpaces, deviceLabel, parseSpaceId, type PrivateDeps } from "./private.js";

const SPACE = "01JDB0ARD00000000000000000";

function setup(stdout = "", code = 0, stderr = "") {
  const run = vi.fn<PrivateDeps["run"]>(() => Promise.resolve({ code, stdout, stderr }));
  const token = vi.fn<() => Promise<string | undefined>>(() => Promise.resolve("at-123"));
  const spaces = new PrivateSpaces(() => "https://vellum.northstar.dev", token, {
    run,
    hostname: () => "ana-mbp",
    pinDir: "/u/private",
  });
  return { run, token, spaces };
}

describe("private spaces", () => {
  it("reads the device key as JSON", async () => {
    const { run, spaces } = setup('{"public_key":"ab","fingerprint":"cd"}\n');
    expect(await spaces.deviceKey()).toEqual({ publicKey: "ab", fingerprint: "cd" });
    expect(run).toHaveBeenCalledWith(["private", "key", "--json"], {});
  });

  it("enrols with the token in the environment, never on the command line", async () => {
    const { run, spaces } = setup("enrolled\nfingerprint: cd\n");
    expect(await spaces.enrol(SPACE)).toEqual({ lines: ["enrolled", "fingerprint: cd"] });
    const [commandArguments, environment] = run.mock.calls[0] ?? [];
    expect(commandArguments).toEqual([
      "private",
      "enrol",
      "--space",
      SPACE,
      "--label=ana-mbp (Vellum desktop)",
    ]);
    expect(commandArguments?.join(" ")).not.toContain("at-123");
    expect(environment).toEqual({
      VELLUM_SERVER: "https://vellum.northstar.dev",
      VELLUM_TOKEN: "at-123",
      VELLUM_PRIVATE_DIR: "/u/private",
    });
  });

  it("reseals and reports the binary's error", async () => {
    const ok = setup("1 resealed\n");
    expect(await ok.spaces.reseal(SPACE)).toEqual({ lines: ["1 resealed"] });
    await ok.spaces.reseal(SPACE, ["0123456789abcdef0123456789abcdef"]);
    expect(ok.run.mock.lastCall?.[0]).toEqual([
      "private",
      "reseal",
      "--space",
      SPACE,
      "--trust=0123456789abcdef0123456789abcdef",
    ]);
    await expect(ok.spaces.reseal(SPACE, ["--server=evil"])).rejects.toThrow(TypeError);
    await expect(ok.spaces.reseal(SPACE, "abc")).rejects.toThrow(TypeError);
    const failing = setup("", 1, "warn\nconflict (409): already enrolled\n");
    await expect(failing.spaces.enrol(SPACE, "Laptop")).rejects.toThrow(/already enrolled/);
  });

  it("needs a signed-in account", async () => {
    const { token, run, spaces } = setup();
    // eslint-disable-next-line unicorn/no-useless-undefined
    token.mockResolvedValueOnce(undefined);
    await expect(spaces.reseal(SPACE)).rejects.toThrow(/Sign in/);
    expect(run).not.toHaveBeenCalled();
  });

  it("refuses space IDs and labels that could reach the command line unchecked", () => {
    for (const bad of [5, "", "../x", `${SPACE} --server x`, "01jdb0ard00000000000000000"]) {
      expect(() => parseSpaceId(bad)).toThrow(TypeError);
    }
    expect(parseSpaceId(SPACE)).toBe(SPACE);
    expect(deviceLabel("Ana\n--server evil", "h")).toBe("Ana --server evil");
    expect(deviceLabel("x".repeat(200), "h")).toHaveLength(80);
    expect(() => deviceLabel(" \u0007 ", "h")).toThrow(TypeError);
  });
});
