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

import { ROUTES_PATH, type RouteStat } from "../requests/api";

import { RoutesPage } from "./routes-page";

const SLOW: RouteStat = {
  method: "GET",
  route: "/api/v1/pages/{page_id}",
  rate_per_s: 2.5,
  error_ratio: 0.004,
  p50_ms: 40,
  p95_ms: 210,
  p99_ms: 480,
  in_flight: 1,
  counts: { requests: 2250, errors: 9 },
  budget_ms: 150,
  over_budget: true,
};

const FAST: RouteStat = {
  method: "POST",
  route: "/api/v1/pages",
  rate_per_s: 0.1,
  error_ratio: 0,
  p50_ms: 8,
  in_flight: 0,
  counts: { requests: 90, errors: 0 },
  over_budget: false,
};

function server(routes: RouteStat[]) {
  return vi.fn(() =>
    Promise.resolve(Response.json({ minutes: 15, routes, in_flight: 1 })),
  ) as unknown as typeof fetch & ReturnType<typeof vi.fn>;
}

afterEach(cleanup);

describe("RoutesPage", () => {
  it("marks routes over their p95 budget", async () => {
    const fetcher = server([SLOW, FAST]);
    render(<RoutesPage fetcher={fetcher} />);
    expect(
      await screen.findByText("1 route is over the p95 budget.", {
        exact: false,
      }),
    ).toBeTruthy();
    const slow = screen.getByRole("row", {
      name: /GET \/api\/v1\/pages\/\{page_id\}/,
    });
    expect(slow.textContent).toContain("210 ms over budget");
    expect(slow.textContent).toContain("0.40 %");
    expect(slow.textContent).toContain("150 ms");
    const fast = screen.getByRole("row", { name: /POST \/api\/v1\/pages/ });
    expect(fast.textContent).toContain("8.0 ms");
    expect(fast.textContent).not.toContain("over budget");
    expect(fetcher).toHaveBeenCalledWith(
      `${ROUTES_PATH}?minutes=15`,
      expect.any(Object),
    );
    fireEvent.change(screen.getByLabelText("Window"), {
      target: { value: "60" },
    });
    await waitFor(() => {
      expect(fetcher).toHaveBeenLastCalledWith(
        `${ROUTES_PATH}?minutes=60`,
        expect.any(Object),
      );
    });
  });

  it(
    "says when every route is within budget, and when there is no " + "traffic",
    async () => {
      render(
        <RoutesPage fetcher={server([FAST, { ...FAST, method: "GET" }])} />,
      );
      expect(
        await screen.findByText(/Every route is within its p95 budget/),
      ).toBeTruthy();
      cleanup();
      render(<RoutesPage fetcher={server([])} />);
      expect(
        await screen.findByText("No requests in the last 15 minutes."),
      ).toBeTruthy();
    },
  );

  it("shows failures", async () => {
    render(
      <RoutesPage
        fetcher={
          vi.fn(() =>
            Promise.resolve(new Response("", { status: 403 })),
          ) as unknown as typeof fetch
        }
      />,
    );
    const denied = await screen.findByRole("alert");
    expect(denied.textContent).toMatch(
      /the server answered 403 \(request .+\)/,
    );
    cleanup();
    render(
      <RoutesPage
        fetcher={
          vi.fn(() => Promise.reject(new Error("x"))) as unknown as typeof fetch
        }
      />,
    );
    const offline = await screen.findByRole("alert");
    expect(offline.textContent).toContain("network");
  });
});
