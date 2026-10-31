// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { memoryDisk } from "../test-support/memory-disk.js";

import { TokenStore, type Tokens, type Vault } from "./token-store.js";

const tokens: Tokens = {
  server: "https://vellum.northstar.dev",
  account: "ana@northstar.dev",
  accessToken: "at",
  refreshToken: "rt",
  expiresAt: 1000,
};

function vault(backend = "os", available = true): Vault {
  return {
    available: () => available,
    backend: () => backend,
    encrypt: (plain) => new TextEncoder().encode(`sealed:${plain}`),
    decrypt: (sealed) => {
      const text = new TextDecoder().decode(sealed);
      if (!text.startsWith("sealed:")) throw new Error("bad key");
      return text.slice("sealed:".length);
    },
  };
}

describe("TokenStore", () => {
  it("keeps tokens sealed on disk across restarts", async () => {
    const disk = memoryDisk();
    await new TokenStore(vault(), disk).save(tokens);
    expect(new TextDecoder().decode(disk.bytes)).toMatch(/^sealed:/);
    expect(await new TokenStore(vault(), disk).load()).toEqual(tokens);
  });

  it("keeps tokens in memory only without a real keychain", async () => {
    const disk = memoryDisk();
    const store = new TokenStore(vault("basic_text"), disk);
    expect(store.persistent).toBe(false);
    await store.save(tokens);
    expect(disk.bytes).toBeUndefined();
    expect(await store.load()).toEqual(tokens);
    expect(
      await new TokenStore(vault("os", false), memoryDisk()).load(),
    ).toBeUndefined();
  });

  it("drops a file it cannot decrypt or understand", async () => {
    const unreadable = memoryDisk("garbage");
    expect(await new TokenStore(vault(), unreadable).load()).toBeUndefined();
    expect(unreadable.bytes).toBeUndefined();
    expect(
      await new TokenStore(vault(), memoryDisk('sealed:{"server":1}')).load(),
    ).toBeUndefined();
    expect(
      await new TokenStore(vault(), memoryDisk("sealed:null")).load(),
    ).toBeUndefined();
    expect(await new TokenStore(vault(), memoryDisk()).load()).toBeUndefined();
  });

  it("forgets tokens on clear", async () => {
    const disk = memoryDisk();
    const store = new TokenStore(vault(), disk);
    await store.save(tokens);
    await store.clear();
    expect(await store.load()).toBeUndefined();
    expect(disk.bytes).toBeUndefined();
  });
});
