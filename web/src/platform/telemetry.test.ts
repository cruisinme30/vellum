// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { newRequestId, newTraceparent, setTelemetrySink, telemetry } from "./telemetry";

describe("telemetry", () => {
  it("defaults to performance measures and swaps sinks", () => {
    const measure = vi.spyOn(performance, "measure");
    telemetry().histogram("vellum.test_ms", 5);
    telemetry().count("vellum.test");
    expect(measure).toHaveBeenCalledWith("vellum.test_ms", expect.any(Object));
    const custom = { histogram: vi.fn(), count: vi.fn() };
    const previous = setTelemetrySink(custom);
    telemetry().count("x");
    expect(custom.count).toHaveBeenCalledWith("x");
    expect(setTelemetrySink(previous)).toBe(custom);
    measure.mockRestore();
  });

  it("makes distinct request IDs and valid traceparents", () => {
    expect(newRequestId()).not.toBe(newRequestId());
    expect(newTraceparent()).toMatch(/^00-[\da-f]{32}-[\da-f]{16}-01$/);
  });
});
