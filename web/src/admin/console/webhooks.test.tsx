// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../../api/http";
import { I18nProvider } from "../../i18n/i18n";

import type { AdminConsoleApi } from "./api";
import { AdminConsoleProvider } from "./context";
import {
  createHttpWebhooksApi,
  type DeliveryView,
  type HookView,
  type WebhooksApi,
} from "./webhooks-api";
import { WebhooksPage } from "./webhooks-page";
import { parseTeams } from "./webhooks-parts";

const EVENT_TYPES = ["issue", "comment", "project"];

const DELIVERY: DeliveryView = {
  delivery_id: "d1",
  event_id: "e1",
  event: "issue.create",
  attempt: 1,
  at: "2026-10-29T10:00:00Z",
  status: 204,
  outcome: "delivered",
  duration_ms: 40,
};

function hook(overrides: Partial<HookView> = {}): HookView {
  return {
    id: "wh_1",
    url: "https://hooks.northstar.dev/vellum",
    description: "CI bridge",
    event_types: ["issue"],
    teams: [],
    enabled: true,
    secret_hint: "abcd",
    rotating: false,
    consecutive_failures: 0,
    last_delivery: DELIVERY,
    queued: 0,
    created_at: "2026-10-29T09:00:00Z",
    ...overrides,
  };
}

function fakeHooks(hooks: HookView[]) {
  const create = vi.fn<WebhooksApi["create"]>((input) =>
    Promise.resolve({
      hook: hook({ id: "wh_2", url: input.url }),
      secret: "whsec_NEWSECRET",
    }),
  );
  const update = vi.fn<WebhooksApi["update"]>((_id, input, enabled) =>
    Promise.resolve(hook({ ...input, enabled })),
  );
  const test = vi.fn<WebhooksApi["test"]>(() =>
    Promise.resolve({ ...DELIVERY, event: "ping" }),
  );
  const redeliver = vi.fn<WebhooksApi["redeliver"]>(() => Promise.resolve());
  const api: WebhooksApi = {
    list: () => Promise.resolve({ hooks, event_types: EVENT_TYPES }),
    create,
    update,
    remove: () => Promise.resolve(),
    rotate: () => Promise.resolve({ hook: hook(), secret: "whsec_ROTATED" }),
    test,
    deliveries: () => Promise.resolve([DELIVERY]),
    redeliver,
  };
  return { api, create, update, test, redeliver };
}

function renderWith(webhooks?: WebhooksApi) {
  const user = userEvent.setup();
  const api = (webhooks === undefined
    ? {}
    : { webhooks }) as unknown as AdminConsoleApi;
  render(
    <I18nProvider>
      <AdminConsoleProvider api={api}>
        <WebhooksPage />
      </AdminConsoleProvider>
    </I18nProvider>,
  );
  return user;
}

describe("webhooks admin", () => {
  it("lists hooks with their health", async () => {
    renderWith(
      fakeHooks([
        hook(),
        hook({
          id: "wh_p",
          enabled: false,
          paused_reason: "failing since Monday",
          teams: ["ENG"],
        }),
      ]).api,
    );
    expect(
      await screen.findAllByText("https://hooks.northstar.dev/vellum"),
    ).toHaveLength(2);
    expect(screen.getByText(/Paused: failing since Monday/u)).toBeTruthy();
    expect(screen.getAllByText("Every public team")).toHaveLength(1);
    expect(screen.getAllByText(/whsec_…abcd/u)).toHaveLength(2);
  });

  it("adds a hook and shows the secret once", async () => {
    const fake = fakeHooks([]);
    const user = renderWith(fake.api);
    expect(await screen.findByText("No webhooks yet.")).toBeTruthy();
    await user.type(
      screen.getByLabelText(/^Receiver URL/u),
      "https://ci.northstar.dev/in",
    );
    await user.click(screen.getByRole("checkbox", { name: "comment" }));
    await user.type(screen.getByLabelText(/^Teams/u), "eng, ops eng");
    await user.click(screen.getByRole("button", { name: "Add webhook" }));
    expect(fake.create).toHaveBeenCalledWith({
      url: "https://ci.northstar.dev/in",
      description: "",
      event_types: ["issue", "comment"],
      teams: ["ENG", "OPS"],
    });
    const secret = await screen.findByRole("region", {
      name: /Copy the signing secret now/u,
    });
    expect(within(secret).getByText("whsec_NEWSECRET")).toBeTruthy();
    await user.click(
      within(secret).getByRole("button", { name: "I have stored it" }),
    );
    expect(screen.queryByText("whsec_NEWSECRET")).toBeNull();
  });

  it("tests, turns off and shows deliveries", async () => {
    const fake = fakeHooks([hook()]);
    const user = renderWith(fake.api);
    await user.click(
      await screen.findByRole("button", { name: "Send a test" }),
    );
    expect(fake.test).toHaveBeenCalledWith("wh_1");
    expect(
      await screen.findByText(/Test delivery: Delivered 204/u),
    ).toBeTruthy();
    await user.click(await screen.findByRole("button", { name: "Turn off" }));
    expect(fake.update).toHaveBeenCalledWith(
      "wh_1",
      {
        url: hook().url,
        description: "CI bridge",
        event_types: ["issue"],
        teams: [],
      },
      false,
    );
    await user.click(await screen.findByRole("button", { name: "Deliveries" }));
    const table = await screen.findByRole("table");
    await user.click(within(table).getByRole("button", { name: "Send again" }));
    expect(fake.redeliver).toHaveBeenCalledWith("wh_1", "d1");
  });

  it("needs a server", () => {
    renderWith();
    expect(screen.getByText(/needs a Vellum server/u)).toBeTruthy();
  });

  it("parses team keys", () => {
    expect(parseTeams(" eng,ops  ENG\nsec ")).toEqual(["ENG", "OPS", "SEC"]);
    expect(parseTeams("")).toEqual([]);
  });

  it("calls the webhook routes", async () => {
    const get = vi.fn(() => Promise.resolve({ deliveries: [] }));
    const send = vi.fn(() => Promise.resolve({}));
    const api = createHttpWebhooksApi({
      get,
      send,
      forgetSession: vi.fn(),
    } as unknown as Http);
    await api.update(
      "wh 1",
      { url: "u", description: "", event_types: [], teams: [] },
      true,
    );
    expect(send).toHaveBeenLastCalledWith(
      "PUT",
      "/api/v1/admin/webhooks/wh%201",
      {
        url: "u",
        description: "",
        event_types: [],
        teams: [],
        enabled: true,
      },
    );
    await api.redeliver("wh_1", "d1");
    expect(send).toHaveBeenLastCalledWith(
      "POST",
      "/api/v1/admin/webhooks/wh_1/redeliver",
      {
        delivery_id: "d1",
      },
    );
    await api.remove("wh_1");
    expect(send).toHaveBeenLastCalledWith(
      "DELETE",
      "/api/v1/admin/webhooks/wh_1",
    );
    expect(await api.deliveries("wh_1")).toEqual([]);
  });
});
