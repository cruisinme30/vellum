// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/http";
import { AppProviders } from "../app";
import { createMockCollectionsStore } from "../collections/mock";
import { createNorthstarStore, PAGE } from "../data/mock/northstar";
import type { RenderApi } from "../render/api";
import { createTestCollab } from "../test/render-app";

import { PageView } from "./page-view";

function Where() {
  const location = useLocation();
  return <output data-testid="where">{location.search}</output>;
}

function renderPage(path: string, renderApi?: RenderApi) {
  const user = userEvent.setup();
  render(
    <AppProviders
      store={createNorthstarStore()}
      collab={createTestCollab()}
      collections={createMockCollectionsStore()}
      render={renderApi}
    >
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/p/:pageId" element={<PageView />} />
        </Routes>
        <Where />
      </MemoryRouter>
    </AppProviders>,
  );
  return user;
}

function fakeRender(page: RenderApi["page"]) {
  const pageSpy = vi.fn(page);
  const stylesheet = vi.fn(() =>
    Promise.resolve({ css: ".vl-render{}", scope: "vl-render", render_version: 2 }),
  );
  const api: RenderApi = { page: pageSpy, block: vi.fn(), stylesheet };
  return { api, pageSpy, stylesheet };
}

const RENDERED = {
  id: PAGE.runbook,
  title: "Runbook",
  html: '<article class="vl-doc"><h1 id="runbook">Runbook</h1><figure class="vl-figure vl-diagram"><svg viewBox="0 0 10 10" role="img"><rect style="fill:var(--vl-chart-1)" width="4" height="4"></rect></svg></figure></article>',
  outline: [],
  warnings: [],
  stale_figures: 0,
  etag: "sha256-x",
  render_version: 2,
};

describe("the reading view", () => {
  it("shows the server's rendering, and switches to the editor and back", async () => {
    const { api, pageSpy, stylesheet } = fakeRender(() => Promise.resolve(RENDERED));
    const user = renderPage(`/p/${PAGE.runbook}?view=read`, api);
    const reading = await screen.findByRole("region", { name: "Rendered page" });
    expect(reading.classList.contains("vl-render")).toBe(true);
    expect(reading.querySelector("rect")?.style.getPropertyValue("fill")).toBe("var(--vl-chart-1)");
    expect(pageSpy).toHaveBeenCalledWith(PAGE.runbook);
    expect(stylesheet).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Read" }).getAttribute("aria-pressed")).toBe("true");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    expect(
      await screen.findByRole("textbox", { name: "Page body" }, { timeout: 5000 }),
    ).toBeDefined();
    expect(screen.getByTestId("where").textContent).toBe("");

    await user.click(screen.getByRole("button", { name: "Read" }));
    expect(await screen.findByRole("region", { name: "Rendered page" })).toBeDefined();
    expect(screen.getByTestId("where").textContent).toBe("?view=read");
  });

  it("says why a page could not be rendered, with Copy diagnostic", async () => {
    const { api } = fakeRender(() =>
      Promise.reject(new ApiError("internal", "Render failed.", { requestId: "req-3" })),
    );
    renderPage(`/p/${PAGE.runbook}?view=read`, api);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Render failed. (request req-3)");
    expect(screen.getByRole("button", { name: "Copy diagnostic" })).toBeDefined();
  });

  it("has no switch in demo mode, where only the editor can show the page", async () => {
    renderPage(`/p/${PAGE.runbook}?view=read`);
    expect(
      await screen.findByRole("textbox", { name: "Page body" }, { timeout: 5000 }),
    ).toBeDefined();
    expect(screen.queryByRole("group", { name: "Page view" })).toBeNull();
  });
});
