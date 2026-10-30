// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { REQUESTS_PATH } from "./api";
import { SUMMARY, TRACE } from "./fixtures";
import { RequestsPage } from "./requests-page";

function server(options: { failTrace?: boolean; rows?: unknown[] } = {}) {
  return vi.fn((url: string) => {
    if (url.startsWith(`${REQUESTS_PATH}/`)) {
      return Promise.resolve(
        options.failTrace
          ? new Response("", { status: 500 })
          : Response.json(TRACE),
      );
    }
    return Promise.resolve(
      Response.json({
        requests: options.rows ?? [
          SUMMARY,
          {
            seq: 40,
            trace_id: "x",
            started_at_ms: 0,
            duration_us: 5,
            name: "job.render",
            error: true,
            span_count: 1,
            has_client: false,
          },
        ],
      }),
    );
  }) as unknown as typeof fetch & ReturnType<typeof vi.fn>;
}

afterEach(cleanup);

describe("RequestsPage", () => {
  it("lists requests and opens a waterfall with span details", async () => {
    render(<RequestsPage fetcher={server()} />);
    expect(screen.getByRole("status").textContent).toContain(
      "Loading requests",
    );
    const open = await screen.findByRole("button", {
      name: "GET /api/v1/pages/{page_id}",
    });
    expect(screen.getByText("failed")).toBeTruthy();
    fireEvent.click(open);
    const spans = await screen.findByRole("list", { name: "Spans" });
    expect(spans.children).toHaveLength(5);
    expect(screen.getByText(/2 spans not kept/)).toBeTruthy();
    const database = screen.getByRole("button", {
      name: /Server db\.query, 4\.0 ms, failed/,
    });
    expect(database.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(database);
    expect(database.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("SELECT 1")).toBeTruthy();
    expect(screen.getByText(/slow plan/)).toBeTruthy();
    fireEvent.click(database);
    expect(screen.queryByText("SELECT 1")).toBeNull();
  });

  it("applies filters", async () => {
    const fetcher = server({ rows: [] });
    render(<RequestsPage fetcher={fetcher} />);
    await screen.findByText(/No requests match/);
    fireEvent.change(screen.getByLabelText("Route"), {
      target: { value: " /api/v1/x " },
    });
    fireEvent.change(screen.getByLabelText("Slower than (ms)"), {
      target: { value: "250" },
    });
    fireEvent.change(screen.getByLabelText("Request or trace id"), {
      target: { value: "req-9" },
    });
    fireEvent.click(screen.getByLabelText("Errors only"));
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => {
      expect(fetcher).toHaveBeenLastCalledWith(
        REQUESTS_PATH +
          `?route=%2Fapi%2Fv1%2Fx&min_duration_ms=250&errors_only=true&id=req-9`,
        expect.any(Object),
      );
    });
    fireEvent.change(screen.getByLabelText("Slower than (ms)"), {
      target: { value: "abc" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await screen.findByText(/No requests match/);
  });

  it("shows failures with the request id", async () => {
    render(<RequestsPage fetcher={server({ failTrace: true })} />);
    fireEvent.click(
      await screen.findByRole("button", {
        name: "GET /api/v1/pages/{page_id}",
      }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(
      /Could not load the trace: the server answered 500 \(request .+\)/,
    );
  });

  it("shows list failures", async () => {
    const failing = vi.fn(() =>
      Promise.reject(new TypeError("boom")),
    ) as unknown as typeof fetch;
    render(<RequestsPage fetcher={failing} />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("network: TypeError: boom");
  });
});
