// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import { createHttpWorkApi, type MyWorkView, type WorkApi } from "./api";
import { MyWorkPage } from "./my-work-page";

const EMPTY = { sections: [] } as unknown as MyWorkView;

function renderAt(path: string, api: WorkApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ work: api }}>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="my-work" element={<MyWorkPage />} />
          </Routes>
        </MemoryRouter>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

function fakeApi(linkSlack: WorkApi["linkSlack"]): WorkApi {
  return { myWork: () => Promise.resolve(EMPTY), linkSlack } as unknown as WorkApi;
}

describe("linking Slack from Save as task", () => {
  it("asks before linking, then confirms", async () => {
    const linkSlack = vi.fn(() => Promise.resolve());
    const user = renderAt("/my-work?slack_link=T01.U01.99.abc", fakeApi(linkSlack));
    expect(await screen.findByText(/Link the Slack account that sent you here/u)).toBeTruthy();
    expect(linkSlack).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Link Slack" }));
    expect(linkSlack).toHaveBeenCalledWith("T01.U01.99.abc");
    expect(await screen.findByText(/Slack is linked/u)).toBeTruthy();
  });

  it("shows the server's refusal and can be dismissed", async () => {
    const linkSlack = vi.fn(() => Promise.reject(new Error("this Slack link has expired")));
    const user = renderAt("/my-work?slack_link=bad", fakeApi(linkSlack));
    await user.click(await screen.findByRole("button", { name: "Link Slack" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/expired/u);
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("button", { name: "Link Slack" })).toBeNull();
  });

  it("shows nothing without a code", async () => {
    renderAt("/my-work", fakeApi(vi.fn()));
    expect(await screen.findByText(/Nothing to do/u)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Link Slack" })).toBeNull();
  });

  it("posts the code to the server", async () => {
    const send = vi.fn(() => Promise.resolve({}));
    const api = createHttpWorkApi({
      get: vi.fn(),
      send,
      forgetSession: vi.fn(),
    } as unknown as Http);
    await api.linkSlack?.("T01.U01.99.abc");
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/my-work/slack-link", {
      code: "T01.U01.99.abc",
    });
  });
});
