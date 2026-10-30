// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../../i18n/i18n";

import type { AdminConsoleApi, AdminSignInApi, ProviderView, SignInView } from "./api";
import { AdminConsoleProvider } from "./context";
import { SignInPage } from "./sign-in-page";

const unused = () => Promise.reject(new Error("not in this test"));

const OKTA: ProviderView = {
  id: "okta",
  name: "Okta",
  kind: "oidc",
  enabled: true,
  issuer: "https://northstar.okta.com",
  clientId: "vellum",
  hasClientSecret: true,
  allowedDomains: ["northstar.dev"],
  autoCreate: false,
  readOnly: false,
  startUrl: "/auth/sso/start?provider=okta",
};

const BASE: SignInView = {
  secondFactor: "optional",
  maxTokenDays: 90,
  requireSso: false,
  providers: [],
  oidcCallbackUrl: "https://vellum.northstar.dev/auth/sso/callback",
  breakGlassAdmins: 0,
};

function fakeSignIn(initial: SignInView) {
  let view = initial;
  const update = vi.fn<AdminSignInApi["update"]>((change) => {
    view = {
      ...view,
      secondFactor: change.secondFactor ?? view.secondFactor,
      requireSso: change.requireSso ?? view.requireSso,
    };
    return Promise.resolve(view);
  });
  const addProvider = vi.fn<AdminSignInApi["addProvider"]>((input) => {
    const added = { ...OKTA, name: input.name, issuer: input.issuer, clientId: input.clientId };
    view = { ...view, providers: [...view.providers, added] };
    return Promise.resolve(added);
  });
  const updateProvider = vi.fn<AdminSignInApi["updateProvider"]>((id, input) => {
    const changed = { ...OKTA, id, name: input.name };
    view = { ...view, providers: view.providers.map((p) => (p.id === id ? changed : p)) };
    return Promise.resolve(changed);
  });
  const removeProvider = vi.fn<AdminSignInApi["removeProvider"]>((id) => {
    view = { ...view, providers: view.providers.filter((p) => p.id !== id) };
    return Promise.resolve(view);
  });
  const api: AdminSignInApi = {
    settings: () => Promise.resolve(view),
    update,
    resetFactors: unused,
    signOutEverywhere: unused,
    addProvider,
    updateProvider,
    removeProvider,
  };
  return { api, update, addProvider, updateProvider, removeProvider };
}

function renderPage(signIn: AdminSignInApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <MemoryRouter>
        <AdminConsoleProvider api={{ signIn } as unknown as AdminConsoleApi}>
          <SignInPage />
        </AdminConsoleProvider>
      </MemoryRouter>
    </I18nProvider>,
  );
  return user;
}

describe("Admin → Sign-in", () => {
  it("saves who must use two-step verification", async () => {
    const { api, update } = fakeSignIn(BASE);
    const user = renderPage(api);
    expect(await screen.findByText(/at most 90 days/)).toBeDefined();
    await user.click(screen.getByRole("radio", { name: /^Admins/ }));
    const form = screen.getByRole("form", { name: "Two-step verification" });
    await user.click(within(form).getByRole("button", { name: "Save" }));
    expect(update).toHaveBeenCalledWith({ secondFactor: "admins" });
    expect(await screen.findByText("Sign-in settings saved.")).toBeDefined();
  });

  it("adds an OpenID Connect provider and shows the redirect URI", async () => {
    const { api, addProvider } = fakeSignIn(BASE);
    const user = renderPage(api);
    expect(await screen.findByText("https://vellum.northstar.dev/auth/sso/callback")).toBeDefined();
    expect(screen.getByText(/No sign-in providers yet/)).toBeDefined();
    const form = screen.getByRole("form", { name: "Add an OpenID Connect provider" });
    await user.type(within(form).getByLabelText("Name"), "Okta");
    await user.type(within(form).getByLabelText("Issuer URL"), "https://northstar.okta.com");
    await user.type(within(form).getByLabelText("Client ID"), "vellum");
    await user.type(within(form).getByLabelText("Client secret"), "s3cret");
    await user.type(
      within(form).getByLabelText("Email domains that get an account on first sign-in"),
      "northstar.dev, example.org",
    );
    await user.click(within(form).getByRole("button", { name: "Save" }));
    expect(addProvider).toHaveBeenCalledWith({
      name: "Okta",
      issuer: "https://northstar.okta.com",
      clientId: "vellum",
      clientSecret: "s3cret",
      allowedDomains: ["northstar.dev", "example.org"],
      autoCreate: false,
      enabled: true,
    });
    expect(await screen.findByText("Okta saved.")).toBeDefined();
    expect(screen.getByRole("rowheader", { name: "Okta" })).toBeDefined();
  });

  it("changes a provider without resending its secret, and removes it", async () => {
    const { api, updateProvider, removeProvider } = fakeSignIn({ ...BASE, providers: [OKTA] });
    const user = renderPage(api);
    await user.click(await screen.findByRole("button", { name: "Change Okta" }));
    const form = screen.getByRole("form", { name: "Change Okta" });
    const secret = within(form).getByLabelText("Client secret");
    expect(secret.getAttribute("placeholder")).toBe("Stored. Leave blank to keep it.");
    const name = within(form).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Okta SSO");
    await user.click(within(form).getByRole("button", { name: "Save" }));
    const [id, input] = updateProvider.mock.calls[0] ?? [];
    expect(id).toBe("okta");
    expect(input?.name).toBe("Okta SSO");
    expect(input && "clientSecret" in input).toBe(false);
    expect(await screen.findByText("Okta SSO saved.")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Remove Okta SSO" }));
    expect(removeProvider).toHaveBeenCalledWith("okta");
    expect(await screen.findByText(/Okta SSO removed/)).toBeDefined();
  });

  it("requires single sign-on and says how many admins can still use a password", async () => {
    const { api, update } = fakeSignIn({ ...BASE, providers: [OKTA], breakGlassAdmins: 1 });
    const user = renderPage(api);
    expect(await screen.findByText(/One break-glass admin can still sign in/)).toBeDefined();
    const form = screen.getByRole("form", { name: "Require single sign-on" });
    await user.click(within(form).getByRole("checkbox"));
    await user.click(within(form).getByRole("button", { name: "Save" }));
    expect(update).toHaveBeenCalledWith({ requireSso: true });
  });

  it("marks environment providers read-only", async () => {
    const { api } = fakeSignIn({
      ...BASE,
      providers: [{ ...OKTA, id: "env", name: "Northstar", readOnly: true }],
    });
    renderPage(api);
    expect(await screen.findByText("Set by the server's environment")).toBeDefined();
    expect(screen.queryByRole("button", { name: "Change Northstar" })).toBeNull();
  });
});
