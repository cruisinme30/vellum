// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Block IDs, the same format as `vellum_kernel::BlockId`: `blk-` and eight
 * lowercase Crockford base32 characters (40 random bits). They only need to
 * be unique within one document.
 */

const ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz";
const PATTERN = /^blk-[0-9a-hjkmnp-tv-z]{8}$/;

/** Formats the low 40 bits of a number as a block ID. */
export function formatBlockId(value: number): string {
  let rest = Math.floor(Math.abs(value)) % 2 ** 40;
  let body = "";
  for (let index = 0; index < 8; index += 1) {
    body = (ALPHABET[rest % 32] ?? "0") + body;
    rest = Math.floor(rest / 32);
  }
  return `blk-${body}`;
}

/** A fresh random block ID from the platform CSPRNG. */
export function newBlockId(): string {
  const bytes = new Uint8Array(5);
  globalThis.crypto.getRandomValues(bytes);
  const value = bytes.reduce((accumulator, byte) => accumulator * 256 + byte, 0);
  return formatBlockId(value);
}

/** Whether a string is a canonical block ID. */
export function isBlockId(value: unknown): value is string {
  return typeof value === "string" && PATTERN.test(value);
}

/** A stable block ID derived from a string, for demo and test documents. */
export function hashedBlockId(seed: string): string {
  // FNV-1a over UTF-16 code units, widened to 40 bits by a second pass.
  let low = 0x81_1c_9d_c5;
  let high = 0x01_00_01_93;
  for (let index = 0; index < seed.length; index += 1) {
    const code = seed.codePointAt(index) ?? 0;
    low = Math.imul(low ^ code, 0x01_00_01_93) >>> 0;
    high = Math.imul(high ^ (code + index), 0x01_00_01_93) >>> 0;
  }
  return formatBlockId((high & 0xff) * 2 ** 32 + low);
}
