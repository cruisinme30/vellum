// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { ApiError } from "../api/http";

import {
  diagnosticOf,
  diagnosticText,
  failureText,
  recentDiagnostic,
} from "./failure";

describe("failureText", () => {
  it(
    "quotes the request ID and remembers the diagnostic behind the " + "line",
    () => {
      const error = new ApiError("conflict", "The page changed.", {
        status: 409,
        requestId: "req-42",
        route: "PATCH /api/v1/drive/items/1",
      });
      const text = failureText(error);
      expect(text).toBe("The page changed. (request req-42)");
      expect(recentDiagnostic(text)).toMatchObject({
        requestId: "req-42",
        route: "PATCH /api/v1/drive/items/1",
        code: "conflict",
        status: 409,
      });
    },
  );

  it("handles plain errors and anything thrown", () => {
    expect(failureText(new Error("boom"))).toBe("boom");
    expect(failureText("odd")).toBe("odd");
    expect(diagnosticOf("odd").requestId).toBeUndefined();
  });
});

describe("diagnosticText", () => {
  it("lists the request, route and time for support, and nothing else", () => {
    const at = new Date("2026-10-29T23:40:00Z");
    const text = diagnosticText(
      {
        message: "Down",
        requestId: "req-1",
        route: "GET /api/v1/my-work",
        code: "unavailable",
        status: 503,
        at,
      },
      "/my-work",
    );
    expect(text).toBe(
      [
        "Vellum diagnostic",
        "Message: Down",
        "Request ID: req-1",
        "Route: GET /api/v1/my-work",
        "Error: unavailable (HTTP 503)",
        "Time: 2026-10-29T23:40:00.000Z",
        "Screen: /my-work",
        "Client: web",
      ].join("\n"),
    );
  });

  it("says when no server answered", () => {
    const text = diagnosticText({ message: "Offline", at: new Date(0) }, "/");
    expect(text).toContain("Request ID: none (no answer from the server)");
    expect(text).toContain("Route: unknown");
    expect(text).not.toContain("Error:");
  });
});
