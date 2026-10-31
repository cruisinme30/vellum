// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../../i18n/i18n";

import type { SearchStatusView } from "./api";
import { SearchStatusPanel } from "./search-status";

const HYBRID: SearchStatusView = {
  engine: "tantivy",
  model_configured: true,
  hybrid: true,
  model: "all-MiniLM-L6-v2",
  dimensions: 384,
  loaded: false,
  idle_minutes: 15,
  job: "embedding",
  done: 128,
  total: 512,
};

function show(load: () => Promise<SearchStatusView>) {
  render(
    <I18nProvider>
      <SearchStatusPanel load={load} refreshKey={1} />
    </I18nProvider>,
  );
}

afterEach(() => {
  vi.useRealTimers();
});

describe("search status", () => {
  it(
    "names the model and shows the backfill's progress, checking again " +
      "while it runs",
    async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const load = vi
        .fn<() => Promise<SearchStatusView>>()
        .mockResolvedValueOnce(HYBRID)
        .mockResolvedValue({ ...HYBRID, loaded: true, job: "done", done: 512 });
      show(load);
      expect(await screen.findByText(/all-MiniLM-L6-v2/)).toBeDefined();
      expect(screen.getByText(/ranked by meaning and words/)).toBeDefined();
      expect(
        screen.getByText(/dropped after .?15.? minutes unused/),
      ).toBeDefined();
      const bar = screen.getByRole("progressbar", {
        name: "Indexing progress",
      });
      expect(bar.getAttribute("value")).toBe("128");
      expect(bar.getAttribute("max")).toBe("512");

      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000);
      });
      expect(await screen.findByText(/Indexing finished/)).toBeDefined();
      expect(screen.getByText("The model is in memory.")).toBeDefined();
      expect(screen.queryByRole("progressbar")).toBeNull();
      expect(load).toHaveBeenCalledTimes(2);
    },
  );

  it(
    "says how to add a model when there is none, and checks again on " +
      "request",
    async () => {
      const load = vi.fn(() =>
        Promise.resolve<SearchStatusView>({
          engine: "postgres",
          model_configured: false,
          hybrid: false,
          loaded: false,
          job: "idle",
          done: 0,
          total: 0,
        }),
      );
      show(load);
      expect(await screen.findByText(/set VELLUM_EMBEDDINGS/)).toBeDefined();
      expect(screen.getByText(/postgres/)).toBeDefined();
      await userEvent.click(
        screen.getByRole("button", { name: "Check again" }),
      );
      expect(load).toHaveBeenCalledTimes(2);
    },
  );

  it("shows why indexing stopped, and a failed read", async () => {
    show(() =>
      Promise.resolve({
        ...HYBRID,
        hybrid: false,
        job: "failed",
        error: "disk full",
      }),
    );
    expect(
      await screen.findByText(/Indexing stopped: .?disk full/),
    ).toBeDefined();
    expect(screen.getByText(/Ranking by meaning is turned off/)).toBeDefined();
  });

  it("reports a status it cannot read", async () => {
    show(() => Promise.reject(new Error("offline")));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("offline");
  });
});
