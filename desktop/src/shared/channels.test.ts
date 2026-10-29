// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { Channel } from "./channels.js";

describe("channels", () => {
  it("are repeated verbatim in the sandboxed preload", () => {
    const preload = readFileSync(new URL("../preload/preload.cts", import.meta.url), "utf8");
    for (const channel of Object.values(Channel)) expect(preload).toContain(`"${channel}"`);
  });
});
