// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The collab socket's wire format (crates/vellum-collab/src/protocol.rs):
 * the standard Yjs sync and awareness messages plus Vellum's custom tags.
 * A binary frame holds one or more messages back to back, each a lib0
 * variable-length unsigned integer tag followed by its payload.
 *
 * Tag 100 (trace context, client → server) is not sent yet.
 *
 * Written here rather than imported from lib0 so the app does not depend
 * on a transitive package, and so every byte from the server is checked
 * against the frame's length before it is trusted.
 */

const MSG_SYNC = 0;
const MSG_AWARENESS = 1;
const MSG_AUTH = 2;
const MSG_QUERY_AWARENESS = 3;
const SYNC_STEP_1 = 0;
const SYNC_STEP_2 = 1;
const SYNC_UPDATE = 2;
const AUTH_DENIED = 0;

/**
 * Client → server: Done was pressed. Server → client: the version is written.
 */
export const MSG_DONE = 101;
/** Server → client: who holds each awareness client ID. */
export const MSG_PEERS = 102;

/** One message on the collab socket. */
export type WireMessage =
  | { readonly type: "sync-step-1"; readonly stateVector: Uint8Array }
  | { readonly type: "sync-step-2"; readonly update: Uint8Array }
  | { readonly type: "update"; readonly update: Uint8Array }
  | { readonly type: "awareness"; readonly update: Uint8Array }
  | { readonly type: "awareness-query" }
  | { readonly type: "auth"; readonly denied: string | undefined }
  | {
      readonly type: "custom";
      readonly tag: number;
      readonly payload: Uint8Array;
    };

/** One awareness client of a connected editor, as the server vouches for it. */
export interface PeerEntry {
  readonly clientId: number;
  /** The signed-in user's ID. */
  readonly user: string;
  readonly canWrite: boolean;
}

/** The frame is not a valid sequence of messages. */
export class WireError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WireError";
  }
}

class Writer {
  private readonly bytes: number[] = [];

  uint(value: number): this {
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new WireError(`not an unsigned integer: ${String(value)}`);
    }
    let rest = value;
    // Division, not bit shifts: client IDs and lengths may exceed 32 bits.
    while (rest >= 0x80) {
      this.bytes.push((rest % 0x80) | 0x80);
      rest = Math.floor(rest / 0x80);
    }
    this.bytes.push(rest);
    return this;
  }

  buffer(value: Uint8Array): this {
    this.uint(value.length);
    for (const byte of value) {
      this.bytes.push(byte);
    }
    return this;
  }

  string(value: string): this {
    return this.buffer(new TextEncoder().encode(value));
  }

  done(): Uint8Array<ArrayBuffer> {
    return Uint8Array.from(this.bytes);
  }
}

class Reader {
  private offset = 0;

  constructor(private readonly bytes: Uint8Array) {}

  get more(): boolean {
    return this.offset < this.bytes.length;
  }

  uint(): number {
    let value = 0;
    let scale = 1;
    for (;;) {
      if (this.offset >= this.bytes.length) {
        throw new WireError("frame ends inside a number");
      }
      const byte = this.bytes[this.offset++] ?? 0;
      value += (byte & 0x7f) * scale;
      if (!Number.isSafeInteger(value)) {
        throw new WireError("number too large");
      }
      if (byte < 0x80) {
        return value;
      }
      scale *= 0x80;
    }
  }

  buffer(): Uint8Array {
    const length = this.uint();
    if (length > this.bytes.length - this.offset) {
      throw new WireError("frame ends inside a payload");
    }
    const value = this.bytes.slice(this.offset, this.offset + length);
    this.offset += length;
    return value;
  }

  string(): string {
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(this.buffer());
    } catch (error) {
      if (error instanceof WireError) {
        throw error;
      }
      throw new WireError("text is not UTF-8");
    }
  }
}

function writeMessage(writer: Writer, message: WireMessage): void {
  switch (message.type) {
    case "sync-step-1": {
      writer.uint(MSG_SYNC).uint(SYNC_STEP_1).buffer(message.stateVector);
      break;
    }
    case "sync-step-2": {
      writer.uint(MSG_SYNC).uint(SYNC_STEP_2).buffer(message.update);
      break;
    }
    case "update": {
      writer.uint(MSG_SYNC).uint(SYNC_UPDATE).buffer(message.update);
      break;
    }
    case "awareness": {
      writer.uint(MSG_AWARENESS).buffer(message.update);
      break;
    }
    case "awareness-query": {
      writer.uint(MSG_QUERY_AWARENESS);
      break;
    }
    case "auth": {
      writer.uint(MSG_AUTH);
      if (message.denied === undefined) {
        writer.uint(1);
      } else {
        writer.uint(AUTH_DENIED).string(message.denied);
      }
      break;
    }
    case "custom": {
      // yrs writes a custom tag as one raw byte, which matches the varint
      // only below 128; Vellum's tags stay in that range.
      if (message.tag <= MSG_QUERY_AWARENESS || message.tag > 127) {
        throw new WireError(`tag ${String(message.tag)} is not a custom tag`);
      }
      writer.uint(message.tag).buffer(message.payload);
      break;
    }
  }
}

function readMessage(reader: Reader): WireMessage {
  const tag = reader.uint();
  switch (tag) {
    case MSG_SYNC: {
      const step = reader.uint();
      const payload = reader.buffer();
      switch (step) {
        case SYNC_STEP_1: {
          return { type: "sync-step-1", stateVector: payload };
        }
        case SYNC_STEP_2: {
          return { type: "sync-step-2", update: payload };
        }
        case SYNC_UPDATE: {
          return { type: "update", update: payload };
        }
        default: {
          throw new WireError(`unknown sync step ${String(step)}`);
        }
      }
    }
    case MSG_AWARENESS: {
      return { type: "awareness", update: reader.buffer() };
    }
    case MSG_AUTH: {
      const answer = reader.uint();
      return {
        type: "auth",
        denied: answer === AUTH_DENIED ? reader.string() : undefined,
      };
    }
    case MSG_QUERY_AWARENESS: {
      return { type: "awareness-query" };
    }
    default: {
      return { type: "custom", tag, payload: reader.buffer() };
    }
  }
}

/** Encodes messages into one frame. */
export function encodeFrame(
  messages: readonly WireMessage[],
): Uint8Array<ArrayBuffer> {
  const writer = new Writer();
  for (const message of messages) {
    writeMessage(writer, message);
  }
  return writer.done();
}

/**
 * Decodes every message in a frame.
 *
 * @throws {WireError} when the frame ends mid-message or a field is malformed.
 */
export function decodeFrame(frame: Uint8Array): WireMessage[] {
  const reader = new Reader(frame);
  const messages: WireMessage[] = [];
  while (reader.more) {
    messages.push(readMessage(reader));
  }
  return messages;
}

/** Encodes a {@link MSG_PEERS} payload (the server's `encode_peers`). */
export function encodePeers(entries: readonly PeerEntry[]): Uint8Array {
  const writer = new Writer().uint(entries.length);
  for (const entry of entries) {
    writer
      .uint(entry.clientId)
      .string(entry.user)
      .uint(entry.canWrite ? 1 : 0);
  }
  return writer.done();
}

/**
 * Decodes a {@link MSG_PEERS} payload.
 *
 * @throws {WireError} when the payload is malformed.
 */
export function decodePeers(payload: Uint8Array): PeerEntry[] {
  const reader = new Reader(payload);
  const count = reader.uint();
  // Every entry takes at least three bytes, so a larger count is a lie.
  if (count * 3 > payload.length) {
    throw new WireError("peer count exceeds the payload");
  }
  const entries: PeerEntry[] = [];
  for (let index = 0; index < count; index++) {
    entries.push({
      clientId: reader.uint(),
      user: reader.string(),
      canWrite: reader.uint() === 1,
    });
  }
  if (reader.more) {
    throw new WireError("trailing bytes after the peer list");
  }
  return entries;
}
