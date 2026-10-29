// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Thin adapters from Node and Electron to the interfaces the tested modules use.

import { spawn } from "node:child_process";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type AddressInfo } from "node:net";
import path from "node:path";

import type { SafeStorage } from "electron";

import type { Disk } from "./settings.js";
import type { Child } from "./sidecar.js";
import type { Vault } from "./token-store.js";

export function fileDisk(file: string): Disk {
  return {
    async read() {
      return readOptionalFile(file);
    },
    async write(bytes) {
      await mkdir(path.dirname(file), { recursive: true });
      await writeFile(file, bytes, { mode: 0o600 });
    },
    async remove() {
      await rm(file, { force: true });
    },
  };
}

export async function readOptionalFile(file: string): Promise<Uint8Array | undefined> {
  try {
    return await readFile(file);
  } catch {
    return undefined;
  }
}

export function safeStorageVault(storage: SafeStorage): Vault {
  return {
    available: () => storage.isEncryptionAvailable(),
    backend: () => (process.platform === "linux" ? storage.getSelectedStorageBackend() : "os"),
    encrypt: (plain) => storage.encryptString(plain),
    decrypt: (sealed) => storage.decryptString(Buffer.from(sealed)),
  };
}

export function spawnChild(
  command: string,
  commandArguments: readonly string[],
  environment: Readonly<Record<string, string>>,
): Child {
  const child = spawn(command, commandArguments, {
    env: { ...process.env, ...environment },
    stdio: ["ignore", "inherit", "inherit"],
  });
  const exited = new Promise<number | null>((resolve) => {
    child.once("exit", resolve);
    child.once("error", () => {
      resolve(-1);
    });
  });
  return {
    exited,
    kill: () => {
      child.kill();
    },
  };
}

export async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  await new Promise<void>((resolve) =>
    server.close(() => {
      resolve();
    }),
  );
  return port;
}
