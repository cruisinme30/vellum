// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Disk } from "../main/settings.js";

/** An in-memory `Disk`, optionally starting with some text. */
export function memoryDisk(
  initial?: string,
): Disk & { bytes: Uint8Array | undefined } {
  const disk: Disk & { bytes: Uint8Array | undefined } = {
    bytes:
      initial === undefined ? undefined : new TextEncoder().encode(initial),
    read: () => Promise.resolve(disk.bytes),
    write: (bytes: Uint8Array) => {
      disk.bytes = bytes;
      return Promise.resolve();
    },
    remove: () => {
      disk.bytes = undefined;
      return Promise.resolve();
    },
  };
  return disk;
}
