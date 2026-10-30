// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import {
  createHttpConnectionsApi,
  type ConnectionsApi,
  type ConnectionsView,
} from "./connections-api";
import { MyWorkConnectionsPage } from "./my-work-connections";

const VIEW: ConnectionsView = {
  caldav_url: "https://vellum.northstar.dev/dav/calendars/priya/",
  email: true,
  slack: true,
  slack_linked: false,
};

function fakeApi(view: ConnectionsView = VIEW) {
  const api = {
    connections: vi.fn(() => Promise.resolve(view)),
    rotateFeed: vi.fn(() =>
      Promise.resolve("https://vellum.northstar.dev/feeds/tasks/abc.ics"),
    ),
    revokeFeed: vi.fn(() => Promise.resolve()),
    rotateCapture: vi.fn(() => Promise.resolve("tasks+xyz@northstar.dev")),
    revokeCapture: vi.fn(() => Promise.resolve()),
    unlinkSlack: vi.fn(() => Promise.resolve()),
    appPassword: vi.fn(() => Promise.resolve("vt_secret")),
    importTasks: vi.fn(() =>
      Promise.resolve({ lists: [{ doc: "L1", title: "Ops", added: 2 }] }),
    ),
  } satisfies ConnectionsApi;
  return api;
}

function renderPage(api?: ConnectionsApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ connections: api }}>
        <MemoryRouter initialEntries={["/my-work/settings"]}>
          <Routes>
            <Route
              path="my-work/settings"
              element={<MyWorkConnectionsPage />}
            />
          </Routes>
        </MemoryRouter>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

describe("My Work settings", () => {
  it("issues and turns off the calendar feed, and makes a CalDAV app password", async () => {
    const api = fakeApi();
    const user = renderPage(api);
    expect(
      await screen.findByRole("textbox", { name: "CalDAV account URL" }),
    ).toHaveProperty("value", VIEW.caldav_url);
    await user.click(
      screen.getByRole("button", { name: "Get a new feed URL" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "Your calendar feed URL" }),
    ).toHaveProperty(
      "value",
      "https://vellum.northstar.dev/feeds/tasks/abc.ics",
    );
    await user.click(screen.getByRole("button", { name: "Turn the feed off" }));
    expect(
      await screen.findByText("The feed is off. Its URL no longer works."),
    ).toBeDefined();
    expect(
      screen.queryByRole("textbox", { name: "Your calendar feed URL" }),
    ).toBeNull();

    await user.click(
      screen.getByRole("button", { name: "Make an app password" }),
    );
    expect(api.appPassword).toHaveBeenCalledWith("CalDAV task app");
    expect(
      await screen.findByRole("textbox", { name: "App password (shown once)" }),
    ).toHaveProperty("value", "vt_secret");
  });

  it("issues a capture address and explains how Slack connects", async () => {
    const api = fakeApi();
    const user = renderPage(api);
    await user.click(
      await screen.findByRole("button", { name: "Get a new address" }),
    );
    expect(
      await screen.findByRole("textbox", { name: "Your capture address" }),
    ).toHaveProperty("value", "tasks+xyz@northstar.dev");
    expect(screen.getByText(/choose Save as task/)).toBeDefined();
    await user.click(
      screen.getByRole("button", { name: "Turn capture by e-mail off" }),
    );
    expect(api.revokeCapture).toHaveBeenCalledOnce();
  });

  it("disconnects a linked Slack account, and says when capture is not set up", async () => {
    const linked = fakeApi({ ...VIEW, slack_linked: true });
    const user = renderPage(linked);
    await user.click(
      await screen.findByRole("button", { name: "Disconnect Slack" }),
    );
    expect(linked.unlinkSlack).toHaveBeenCalledOnce();
    await waitFor(() => {
      expect(linked.connections).toHaveBeenCalledTimes(2);
    });
  });

  it("says what the server does not offer", async () => {
    renderPage(fakeApi({ ...VIEW, email: false, slack: false }));
    expect(
      await screen.findByText(/does not receive capture e-mail/),
    ).toBeDefined();
    expect(screen.getByText(/Slack capture is not set up/)).toBeDefined();
    expect(
      screen.queryByRole("button", { name: "Get a new address" }),
    ).toBeNull();
  });

  it("previews an export, imports it and shows what was added", async () => {
    const api = fakeApi();
    const user = renderPage(api);
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "From" }),
      "todoist",
    );
    const csv = "TYPE,CONTENT\ntask,One\ntask,Two\n";
    await user.upload(
      screen.getByLabelText("Export file"),
      new File([csv], "ops.csv", { type: "text/csv" }),
    );
    expect(await screen.findByText("ops.csv holds 2 tasks.")).toBeDefined();
    await user.type(
      screen.getByRole("textbox", { name: "List name for this project" }),
      "Ops",
    );
    await user.click(screen.getByRole("button", { name: "Import" }));
    expect(api.importTasks).toHaveBeenCalledWith("todoist", csv, "Ops");
    expect(await screen.findByText("Ops: 2 tasks added")).toBeDefined();
  });

  it("will not import a file it cannot read", async () => {
    const user = renderPage(fakeApi());
    await user.upload(
      await screen.findByLabelText("Export file"),
      new File(["nope"], "tasks.json", { type: "application/json" }),
    );
    expect(
      await screen.findByText("tasks.json is not an export from that app."),
    ).toBeDefined();
    expect(screen.getByRole("button", { name: "Import" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("explains itself in demo mode", () => {
    renderPage();
    expect(screen.getByText(/needs a Vellum server/)).toBeDefined();
  });
});

describe("connections API", () => {
  it("calls the My Work, token and import routes", async () => {
    const get = vi.fn(() => Promise.resolve(VIEW));
    const send = vi.fn((_method: string, path: string) =>
      Promise.resolve(
        path === "/api/v1/me/tokens"
          ? { token: "vt_1", details: {} }
          : { value: "v", lists: [] },
      ),
    );
    const api = createHttpConnectionsApi({ get, send } as unknown as Http);
    expect(await api.connections()).toEqual(VIEW);
    expect(get).toHaveBeenCalledWith("/api/v1/my-work/connections");
    expect(await api.rotateFeed()).toBe("v");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/my-work/feed", {});
    await api.revokeCapture();
    expect(send).toHaveBeenLastCalledWith(
      "DELETE",
      "/api/v1/my-work/capture-address",
    );
    await api.unlinkSlack();
    expect(send).toHaveBeenLastCalledWith(
      "DELETE",
      "/api/v1/my-work/slack-link",
    );
    expect(await api.appPassword("CalDAV")).toBe("vt_1");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/me/tokens", {
      name: "CalDAV",
      scopes: ["read", "write:direct"],
    });
    await api.importTasks("things", "[]");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/tasks/import", {
      from: "things",
      content: "[]",
    });
    await api.importTasks("todoist", "csv", " Ops ");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/tasks/import", {
      from: "todoist",
      content: "csv",
      project: "Ops",
    });
  });
});
