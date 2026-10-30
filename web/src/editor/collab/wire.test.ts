// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  decodeFrame,
  decodePeers,
  encodeFrame,
  encodePeers,
  MSG_DONE,
  MSG_PEERS,
  WireError,
  type WireMessage,
} from "./wire";

const bytes = (...values: number[]) => Uint8Array.from(values);

describe("collab wire format", () => {
  it("round-trips every message type in one frame", () => {
    const messages: WireMessage[] = [
      { type: "sync-step-1", stateVector: bytes(0) },
      { type: "sync-step-2", update: bytes(0, 0) },
      { type: "update", update: bytes(1, 2, 3) },
      { type: "awareness", update: bytes(1, 5, 0) },
      { type: "awareness-query" },
      { type: "auth", denied: "no" },
      { type: "auth", denied: undefined },
      { type: "custom", tag: MSG_DONE, payload: bytes() },
      { type: "custom", tag: MSG_PEERS, payload: bytes(0) },
    ];
    expect(decodeFrame(encodeFrame(messages))).toEqual(messages);
  });

  it("writes the bytes yrs writes", () => {
    // MSG_SYNC, step 1, a one-byte state vector holding zero clients.
    expect(
      encodeFrame([{ type: "sync-step-1", stateVector: bytes(0) }]),
    ).toEqual(bytes(0, 0, 1, 0));
    // Done is the raw tag and an empty payload.
    expect(
      encodeFrame([{ type: "custom", tag: MSG_DONE, payload: bytes() }]),
    ).toEqual(bytes(101, 0));
    // A two-byte varint length.
    const long = new Uint8Array(200);
    expect(
      encodeFrame([{ type: "update", update: long }]).subarray(0, 4),
    ).toEqual(bytes(0, 2, 0xc8, 0x01));
  });

  it("refuses tags yrs would write differently", () => {
    expect(() =>
      encodeFrame([{ type: "custom", tag: 200, payload: bytes() }]),
    ).toThrow(WireError);
    expect(() =>
      encodeFrame([{ type: "custom", tag: 1, payload: bytes() }]),
    ).toThrow(WireError);
  });

  it.each([
    ["a number cut off", bytes(0x80)],
    ["a payload longer than the frame", bytes(0, 0, 5, 1, 2)],
    ["an unknown sync step", bytes(0, 9, 0)],
    ["a custom message without its payload", bytes(101)],
    [
      "a number past 2^53",
      bytes(0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f),
    ],
  ])("refuses %s", (_name, frame) => {
    expect(() => decodeFrame(frame)).toThrow(WireError);
  });

  it("decodes an empty frame as no messages", () => {
    expect(decodeFrame(bytes())).toEqual([]);
  });
});

describe("peer lists", () => {
  it(
    "round-trips the server's encode_peers, with client IDs past 32 " + "bits",
    () => {
      const entries = [
        { clientId: 7, user: "01J0000000000000000000USER", canWrite: true },
        {
          clientId: 2 ** 40 + 3,
          user: "01J0000000000000000000VIEW",
          canWrite: false,
        },
      ];
      expect(decodePeers(encodePeers(entries))).toEqual(entries);
    },
  );

  it("matches the server's layout byte for byte", () => {
    // count 1, client 5, user "u", can_write 1
    expect(encodePeers([{ clientId: 5, user: "u", canWrite: true }])).toEqual(
      bytes(1, 5, 1, 0x75, 1),
    );
  });

  it.each([
    ["a count larger than the payload", bytes(200, 1)],
    ["trailing bytes", bytes(0, 1)],
    ["text that is not UTF-8", bytes(1, 5, 1, 0xff, 1)],
    ["an entry cut off", bytes(1, 5, 1, 0x75)],
  ])("refuses %s", (_name, payload) => {
    expect(() => decodePeers(payload)).toThrow(WireError);
  });
});
