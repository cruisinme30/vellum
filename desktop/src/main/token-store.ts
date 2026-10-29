// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Disk } from "./settings.js";

/** A signed-in session for one server. */
export interface Tokens {
  readonly server: string;
  readonly account: string;
  readonly accessToken: string;
  readonly refreshToken: string | undefined;
  /** Milliseconds since the epoch. */
  readonly expiresAt: number;
}

/** The OS keychain, as Electron's `safeStorage` exposes it. */
export interface Vault {
  available(): boolean;
  /** Linux only: `basic_text` means no keyring, so encryption is cosmetic. */
  backend(): string;
  encrypt(plain: string): Uint8Array;
  decrypt(sealed: Uint8Array): string;
}

/**
 * Keeps tokens encrypted by the OS keychain (Keychain, DPAPI, libsecret or
 * KWallet). Without a real keychain it keeps them in memory only, so the
 * person signs in again after a restart rather than leaving a readable
 * token on disk (ADR 0023, D99).
 */
export class TokenStore {
  private memory: Tokens | undefined;

  constructor(
    private readonly vault: Vault,
    private readonly disk: Disk,
  ) {}

  /** Whether tokens survive a restart. */
  get persistent(): boolean {
    return this.vault.available() && this.vault.backend() !== "basic_text";
  }

  async load(): Promise<Tokens | undefined> {
    if (this.memory !== undefined || !this.persistent) return this.memory;
    const sealed = await this.disk.read();
    if (sealed === undefined) return undefined;
    try {
      const value: unknown = JSON.parse(this.vault.decrypt(sealed));
      this.memory = isTokens(value) ? value : undefined;
    } catch {
      // A keychain reset makes the file undecryptable: sign in again.
      await this.disk.remove();
    }
    return this.memory;
  }

  async save(tokens: Tokens): Promise<void> {
    this.memory = tokens;
    if (this.persistent) await this.disk.write(this.vault.encrypt(JSON.stringify(tokens)));
  }

  async clear(): Promise<void> {
    this.memory = undefined;
    await this.disk.remove();
  }
}

function isTokens(value: unknown): value is Tokens {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record["server"] === "string" &&
    typeof record["account"] === "string" &&
    typeof record["accessToken"] === "string" &&
    (record["refreshToken"] === undefined || typeof record["refreshToken"] === "string") &&
    typeof record["expiresAt"] === "number"
  );
}
