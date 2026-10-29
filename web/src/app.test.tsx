// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { App } from "./app";

afterEach(cleanup);

describe("App", () => {
  it("renders the product name as the main heading", () => {
    render(<App />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Vellum");
  });

  it("mounts Admin → Requests on its path", () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(Response.json({ requests: [] }));
    render(<App path="/admin/requests" />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Requests");
    fetchSpy.mockRestore();
  });

  it("mounts Admin → Routes on its path", () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(Response.json({ minutes: 15, routes: [], in_flight: 0 }));
    render(<App path="/admin/routes" />);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Routes");
    fetchSpy.mockRestore();
  });
});
