// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Keystrokes as y-protocols frames, for the keystroke-to-peers scenario
// (#528, ADR 0022 D299). k6 has no Yjs, so this writes the few bytes of a
// Yjs v1 update by hand: one string inserted into a root text named
// `k6probe`. The server applies it like any edit and relays it to every
// other editor in the room; the editor and the Markdown ignore that root.
// No imports, so plain Node can check the bytes against Yjs itself.

const encoder = new TextEncoder();

function varUint(out, value) {
  let n = value;
  while (n > 0x7f) {
    out.push(0x80 | (n & 0x7f));
    n = Math.floor(n / 128);
  }
  out.push(n);
}

function varString(out, text) {
  const bytes = encoder.encode(text);
  varUint(out, bytes.length);
  for (const b of bytes) out.push(b);
}

/** The root text the probe writes to. */
export const PROBE_ROOT = "k6probe";

/**
 * A Yjs v1 update from `client` inserting `text` (ASCII) at the start of
 * the probe root. `clock` is how many characters this client inserted
 * before; the next call passes `clock + text.length`.
 */
export function insertUpdate(client, clock, text) {
  const out = [];
  varUint(out, 1); // one client
  varUint(out, 1); // one struct
  varUint(out, client);
  varUint(out, clock);
  out.push(4); // Item with string content, no origins, no parent sub
  varUint(out, 1); // the parent is a root, named by key
  varString(out, PROBE_ROOT);
  varString(out, text);
  varUint(out, 0); // empty delete set
  return out;
}

/** The sync "update" frame carrying `update`. */
export function updateFrame(update) {
  const out = [0, 2]; // sync message, update
  varUint(out, update.length);
  return new Uint8Array(out.concat(update)).buffer;
}

/** The text of a probe keystroke: who sent it and when (ms). */
export function stamp(pair, sentMs) {
  return `<k6|${pair}|${sentMs}>`;
}

/** Every stamp for `pair` in a frame, as send times (ms). */
export function stampsIn(frame, pair) {
  const bytes = new Uint8Array(frame);
  let text = "";
  for (const b of bytes) text += String.fromCharCode(b);
  const found = [];
  const pattern = new RegExp(`<k6\\|${pair}\\|(\\d+)>`, "g");
  let match = pattern.exec(text);
  while (match !== null) {
    found.push(Number(match[1]));
    match = pattern.exec(text);
  }
  return found;
}
