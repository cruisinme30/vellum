// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { AdminConsoleApi, AdminTokensApi } from "../admin/console/api";
import { AdminConsoleProvider } from "../admin/console/context";
import { AdminTokensPage } from "../admin/console/tokens-page";
import type { Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import { createHttpAccountApi, type AccountApi, type TokenView } from "./api";
import { ApiTokensPage, defaultExpiry, expiryChoices } from "./api-tokens-page";

const CI: TokenView = {
  id: "tok1",
  name: "CI",
  scopes: ["read", "work"],
  createdAt: "2026-10-01T10:00:00Z",
  expiresAt: "2099-01-01T00:00:00Z",
};

function fakeAccount(overrides: Partial<AccountApi> = {}) {
  const createToken = vi.fn<AccountApi["createToken"]>((input) =>
    Promise.resolve({
      token: "vlm_secret_once",
      details: { ...CI, id: "tok2", name: input.name, scopes: [...input.scopes] },
    }),
  );
  const revokeToken = vi.fn<AccountApi["revokeToken"]>(() => Promise.resolve());
  const api: AccountApi = {
    tokens: () => Promise.resolve([CI]),
    tokenPolicy: () => Promise.resolve({ maxDays: 90, scopes: ["read", "write:draft", "work"] }),
    createToken,
    revokeToken,
    ...overrides,
  };
  return { api, createToken, revokeToken };
}

function renderTokens(api?: AccountApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ account: api }}>
        <ApiTokensPage />
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

describe("expiry choices", () => {
  it("offer never only without a cap, and always the cap itself", () => {
    expect(expiryChoices()).toEqual([7, 30, 90, 365, "never"]);
    expect(expiryChoices(90)).toEqual([7, 30, 90]);
    expect(expiryChoices(45)).toEqual([7, 30, 45]);
    expect(expiryChoices(3)).toEqual([3]);
    expect(defaultExpiry()).toBe(30);
    expect(defaultExpiry(14)).toBe(14);
  });
});

describe("account API", () => {
  it("calls the token routes", async () => {
    const get = vi.fn(() => Promise.resolve([]));
    const send = vi.fn(() => Promise.resolve({}));
    const http = { get, send, forgetSession: vi.fn() } as unknown as Http;
    const api = createHttpAccountApi(http);
    await api.tokens();
    await api.tokenPolicy();
    await api.createToken({ name: "CI", scopes: ["read"], expiresInDays: 30 });
    await api.revokeToken("a/b");
    expect(get).toHaveBeenCalledWith("/api/v1/me/tokens");
    expect(get).toHaveBeenCalledWith("/api/v1/me/token-policy");
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/me/tokens", {
      name: "CI",
      scopes: ["read"],
      expiresInDays: 30,
    });
    expect(send).toHaveBeenCalledWith("DELETE", "/api/v1/me/tokens/a%2Fb");
  });
});

describe("Settings → API tokens", () => {
  it("needs a server", () => {
    renderTokens();
    expect(screen.getByText(/needs a Vellum server/i)).toBeDefined();
  });

  it("lists tokens and creates one within the cap, showing the secret once", async () => {
    const { api, createToken } = fakeAccount();
    const user = renderTokens(api);
    const table = await screen.findByRole("table");
    expect(within(table).getByRole("rowheader", { name: "CI" })).toBeDefined();
    expect(within(table).getByText("Never")).toBeDefined();

    // No admin scope offered to a member, and no "never" under a cap.
    expect(screen.queryByRole("checkbox", { name: /Administer/ })).toBeNull();
    const expiry = screen.getByRole("combobox", { name: /Expires after/ });
    expect(within(expiry).queryByRole("option", { name: "Never" })).toBeNull();
    expect(screen.getByText(/at most 90 days/)).toBeDefined();

    await user.type(screen.getByRole("textbox", { name: "Name" }), "Claude Desktop");
    await user.click(screen.getByRole("checkbox", { name: /Work/ }));
    await user.selectOptions(expiry, "90");
    await user.click(screen.getByRole("button", { name: "Create token" }));
    expect(createToken).toHaveBeenCalledWith({
      name: "Claude Desktop",
      scopes: ["read", "work"],
      expiresInDays: 90,
    });
    const secret = await screen.findByTestId("new-token");
    expect(secret.textContent).toBe("vlm_secret_once");
    await user.click(screen.getByRole("button", { name: "I have stored it" }));
    expect(screen.queryByTestId("new-token")).toBeNull();
  });

  it("refuses a token with no scope before calling the server", async () => {
    const { api, createToken } = fakeAccount();
    const user = renderTokens(api);
    await screen.findByRole("table");
    await user.type(screen.getByRole("textbox", { name: "Name" }), "Nothing");
    await user.click(screen.getByRole("checkbox", { name: /Read/ }));
    await user.click(screen.getByRole("button", { name: "Create token" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Choose at least one scope.");
    expect(createToken).not.toHaveBeenCalled();
  });

  it("revokes after a confirmation", async () => {
    const { api, revokeToken } = fakeAccount();
    const user = renderTokens(api);
    await user.click(await screen.findByRole("button", { name: "Revoke CI" }));
    expect(revokeToken).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Revoke" }));
    expect(revokeToken).toHaveBeenCalledWith("tok1");
    expect(await screen.findByText("CI revoked.")).toBeDefined();
  });
});

describe("Admin → API tokens", () => {
  it("lists everyone's tokens, revokes one and sets the cap", async () => {
    const revoke = vi.fn<AdminTokensApi["revoke"]>(() => Promise.resolve());
    const setPolicy = vi.fn<AdminTokensApi["setPolicy"]>((maxDays) =>
      Promise.resolve({ ...(maxDays === undefined ? {} : { maxDays }), scopes: ["read"] }),
    );
    const tokens: AdminTokensApi = {
      list: () =>
        Promise.resolve([
          { token: CI, owner: { id: "u1", name: "Sam", email: "sam@northstar.dev" } },
        ]),
      revoke,
      policy: () => Promise.resolve({ scopes: ["read"] }),
      setPolicy,
    };
    const user = userEvent.setup();
    render(
      <I18nProvider>
        <AdminConsoleProvider api={{ tokens } as unknown as AdminConsoleApi}>
          <AdminTokensPage />
        </AdminConsoleProvider>
      </I18nProvider>,
    );
    await user.click(await screen.findByRole("button", { name: "Revoke Sam's token CI" }));
    await user.click(screen.getByRole("button", { name: "Revoke" }));
    expect(revoke).toHaveBeenCalledWith("tok1");

    const days = await screen.findByRole("spinbutton", { name: /Days/ });
    expect((days as HTMLInputElement).disabled).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: /Limit how long/ }));
    await user.clear(days);
    await user.type(days, "30");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(setPolicy).toHaveBeenCalledWith(30);
    expect(await screen.findByText(/at most 30 days/)).toBeDefined();
  });
});
