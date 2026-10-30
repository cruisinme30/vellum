// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../../api/http";
import { I18nProvider } from "../../i18n/i18n";

import type {
  AdminConsoleApi,
  AdminSignInApi,
  ProviderView,
  SignInView,
} from "./api";
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
  certificateFingerprints: [],
};

const SAML: ProviderView = {
  id: "entra",
  name: "Entra ID",
  kind: "saml",
  enabled: true,
  hasClientSecret: false,
  allowedDomains: [],
  autoCreate: false,
  readOnly: false,
  startUrl: "/auth/sso/start?provider=entra",
  certificateFingerprints: ["AB:CD"],
};

const BASE: SignInView = {
  secondFactor: "optional",
  maxTokenDays: 90,
  requireSso: false,
  providers: [],
  oidcCallbackUrl: "https://vellum.northstar.dev/auth/sso/callback",
  breakGlassAdmins: 0,
  saml: {
    entityId: "https://vellum.northstar.dev/saml/metadata",
    acsUrl: "https://vellum.northstar.dev/saml/acs",
  },
  scim: { enabled: false, baseUrl: "https://vellum.northstar.dev/scim/v2" },
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
    const added: ProviderView =
      input.kind === "saml"
        ? { ...SAML, name: input.name, entityId: input.entityId ?? "" }
        : {
            ...OKTA,
            name: input.name,
            issuer: input.issuer ?? "",
            clientId: input.clientId ?? "",
          };
    view = { ...view, providers: [...view.providers, added] };
    return Promise.resolve(added);
  });
  const updateProvider = vi.fn<AdminSignInApi["updateProvider"]>(
    (id, input) => {
      const changed = { ...OKTA, id, name: input.name };
      view = {
        ...view,
        providers: view.providers.map((p) => (p.id === id ? changed : p)),
      };
      return Promise.resolve(changed);
    },
  );
  const removeProvider = vi.fn<AdminSignInApi["removeProvider"]>((id) => {
    view = { ...view, providers: view.providers.filter((p) => p.id !== id) };
    return Promise.resolve(view);
  });
  const rotateScim = vi.fn<AdminSignInApi["rotateScim"]>(() => {
    const scim = {
      enabled: true,
      tokenCreatedAt: "2026-10-30T08:00:00Z",
      baseUrl: "https://vellum.northstar.dev/scim/v2",
    };
    view = { ...view, scim };
    return Promise.resolve({ token: "vsc_new-token", scim });
  });
  const revokeScim = vi.fn<AdminSignInApi["revokeScim"]>(() => {
    view = { ...view, scim: { enabled: false, baseUrl: view.scim.baseUrl } };
    return Promise.resolve(view);
  });
  const samlMetadata = vi.fn<AdminSignInApi["samlMetadata"]>(() =>
    Promise.resolve({
      entityId: "https://sts.windows.net/tenant/",
      ssoUrl: "https://login.microsoftonline.com/tenant/saml2",
      certificate:
        "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
      certificateFingerprints: ["AB:CD"],
    }),
  );
  const api: AdminSignInApi = {
    settings: () => Promise.resolve(view),
    update,
    resetFactors: unused,
    signOutEverywhere: unused,
    addProvider,
    updateProvider,
    removeProvider,
    rotateScim,
    revokeScim,
    samlMetadata,
  };
  return {
    api,
    update,
    addProvider,
    updateProvider,
    removeProvider,
    rotateScim,
    samlMetadata,
  };
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
    expect(
      await screen.findByText("https://vellum.northstar.dev/auth/sso/callback"),
    ).toBeDefined();
    expect(screen.getByText(/No sign-in providers yet/)).toBeDefined();
    const form = screen.getByRole("form", { name: "Add a sign-in provider" });
    await user.type(within(form).getByLabelText("Name"), "Okta");
    await user.type(
      within(form).getByLabelText("Issuer URL"),
      "https://northstar.okta.com",
    );
    await user.type(within(form).getByLabelText("Client ID"), "vellum");
    await user.type(within(form).getByLabelText("Client secret"), "s3cret");
    await user.type(
      within(form).getByLabelText(
        "Email domains that get an account on first sign-in",
      ),
      "northstar.dev, example.org",
    );
    await user.click(within(form).getByRole("button", { name: "Save" }));
    expect(addProvider).toHaveBeenCalledWith({
      kind: "oidc",
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

  it("adds a SAML provider from its metadata URL", async () => {
    const { api, addProvider } = fakeSignIn(BASE);
    const user = renderPage(api);
    expect(
      await screen.findByText("https://vellum.northstar.dev/saml/acs"),
    ).toBeDefined();
    const form = screen.getByRole("form", { name: "Add a sign-in provider" });
    await user.click(within(form).getByRole("radio", { name: "SAML 2.0" }));
    await user.type(within(form).getByLabelText("Name"), "Entra ID");
    await user.type(
      within(form).getByLabelText("Metadata URL"),
      "https://login.microsoftonline.com/tenant/federationmetadata.xml",
    );
    await user.click(
      within(form).getByRole("button", { name: "Read metadata" }),
    );
    expect(
      await within(form).findByText(/with one signing certificate/),
    ).toBeDefined();
    expect(
      within(form).getByLabelText<HTMLInputElement>("Provider entity ID").value,
    ).toBe("https://sts.windows.net/tenant/");
    await user.click(within(form).getByRole("button", { name: "Save" }));
    expect(addProvider).toHaveBeenCalledWith({
      kind: "saml",
      name: "Entra ID",
      entityId: "https://sts.windows.net/tenant/",
      ssoUrl: "https://login.microsoftonline.com/tenant/saml2",
      certificate:
        "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----",
      metadataUrl:
        "https://login.microsoftonline.com/tenant/federationmetadata.xml",
      allowedDomains: [],
      autoCreate: false,
      enabled: true,
    });
    expect(await screen.findByText("Entra ID saved.")).toBeDefined();
  });

  it("changes a provider without resending its secret, and removes it", async () => {
    const { api, updateProvider, removeProvider } = fakeSignIn({
      ...BASE,
      providers: [OKTA],
    });
    const user = renderPage(api);
    await user.click(
      await screen.findByRole("button", { name: "Change Okta" }),
    );
    const form = screen.getByRole("form", { name: "Change Okta" });
    const secret = within(form).getByLabelText("Client secret");
    expect(secret.getAttribute("placeholder")).toBe(
      "Stored. Leave blank to keep it.",
    );
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

  it(
    "requires single sign-on and says how many admins can still use a " +
      "password",
    async () => {
      const { api, update } = fakeSignIn({
        ...BASE,
        providers: [OKTA],
        breakGlassAdmins: 1,
      });
      const user = renderPage(api);
      expect(
        await screen.findByText(/One break-glass admin can still sign in/),
      ).toBeDefined();
      const form = screen.getByRole("form", { name: "Require single sign-on" });
      await user.click(within(form).getByRole("checkbox"));
      await user.click(within(form).getByRole("button", { name: "Save" }));
      expect(update).toHaveBeenCalledWith({ requireSso: true });
    },
  );

  it("turns SCIM on, shows the token once, rotates and turns it off", async () => {
    const { api } = fakeSignIn(BASE);
    const user = renderPage(api);
    expect(
      await screen.findByText("https://vellum.northstar.dev/scim/v2"),
    ).toBeDefined();
    await user.click(
      screen.getByRole("button", { name: "Turn on and make a token" }),
    );
    const shown = await screen.findByTestId("scim-token");
    expect(shown.textContent).toBe("vsc_new-token");
    expect(
      screen.getByText(/On, with a token made on Oct 30, 2026/),
    ).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByTestId("scim-token")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Make a new token" }));
    expect(screen.getByText(/stops working at once/)).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Make a new token" }));
    expect(await screen.findByTestId("scim-token")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Turn off" }));
    expect(await screen.findByText(/Off\. Turn it on/)).toBeDefined();
    expect(screen.queryByTestId("scim-token")).toBeNull();
  });

  it("marks environment providers read-only", async () => {
    const { api } = fakeSignIn({
      ...BASE,
      providers: [{ ...OKTA, id: "env", name: "Northstar", readOnly: true }],
    });
    renderPage(api);
    expect(
      await screen.findByText("Set by the server's environment"),
    ).toBeDefined();
    expect(
      screen.queryByRole("button", { name: "Change Northstar" }),
    ).toBeNull();
  });
});

const refused = () => new ApiError("conflict", "the server said no");

async function textOf(found: Promise<HTMLElement>): Promise<string | null> {
  const element = await found;
  return element.textContent;
}

describe("Admin → Sign-in when the server refuses", () => {
  it("shows a settings page that cannot load", async () => {
    const { api } = fakeSignIn(BASE);
    renderPage({ ...api, settings: () => Promise.reject(refused()) });
    expect(await textOf(screen.findByRole("alert"))).toBe("the server said no");
  });

  it("says why each change failed", async () => {
    const fake = fakeSignIn({ ...BASE, providers: [OKTA] });
    const user = renderPage(fake.api);
    await screen.findByRole("rowheader", { name: "Okta" });

    fake.update.mockRejectedValueOnce(refused());
    const twoStep = screen.getByRole("form", { name: "Two-step verification" });
    await user.click(within(twoStep).getByRole("button", { name: "Save" }));
    expect(await textOf(within(twoStep).findByRole("alert"))).toBe(
      "the server said no",
    );

    fake.update.mockRejectedValueOnce(refused());
    const sso = screen.getByRole("form", { name: "Require single sign-on" });
    await user.click(within(sso).getByRole("button", { name: "Save" }));
    expect(await textOf(within(sso).findByRole("alert"))).toBe(
      "the server said no",
    );

    fake.removeProvider.mockRejectedValueOnce(refused());
    await user.click(screen.getByRole("button", { name: "Remove Okta" }));
    expect(await screen.findAllByText("the server said no")).toHaveLength(3);

    fake.addProvider.mockRejectedValueOnce(refused());
    const add = screen.getByRole("form", { name: "Add a sign-in provider" });
    await user.type(within(add).getByLabelText("Name"), "Okta 2");
    await user.type(
      within(add).getByLabelText("Issuer URL"),
      "https://two.okta.com",
    );
    await user.type(within(add).getByLabelText("Client ID"), "v");
    await user.click(within(add).getByRole("button", { name: "Save" }));
    expect(await textOf(within(add).findByRole("alert"))).toBe(
      "the server said no",
    );
  });

  it(
    "shows a SAML provider's stored certificate, and a metadata URL that " +
      "cannot be read",
    async () => {
      const fake = fakeSignIn({
        ...BASE,
        providers: [
          {
            ...SAML,
            ssoUrl: "https://login.example/saml2",
            entityId: "https://sts/",
          },
        ],
      });
      fake.samlMetadata.mockRejectedValueOnce(refused());
      const user = renderPage(fake.api);
      await user.click(
        await screen.findByRole("button", { name: "Change Entra ID" }),
      );
      const form = screen.getByRole("form", { name: "Change Entra ID" });
      expect(within(form).getByText("AB:CD")).toBeDefined();
      expect(
        within(form)
          .getByLabelText("Signing certificate (PEM)")
          .getAttribute("placeholder"),
      ).toBe("Stored. Paste a new certificate to replace it.");
      await user.type(
        within(form).getByLabelText("Metadata URL"),
        "https://bad.example/md",
      );
      await user.click(
        within(form).getByRole("button", { name: "Read metadata" }),
      );
      expect(await textOf(within(form).findByRole("alert"))).toBe(
        "the server said no",
      );
      await user.click(within(form).getByRole("button", { name: "Cancel" }));
      expect(
        screen.getByRole("form", { name: "Add a sign-in provider" }),
      ).toBeDefined();
    },
  );

  it("keeps the SCIM token when making a new one is cancelled or fails", async () => {
    const fake = fakeSignIn({
      ...BASE,
      scim: {
        enabled: true,
        tokenCreatedAt: "2026-10-30T08:00:00Z",
        baseUrl: BASE.scim.baseUrl,
      },
    });
    const user = renderPage(fake.api);
    await user.click(
      await screen.findByRole("button", { name: "Make a new token" }),
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText(/stops working at once/)).toBeNull();
    fake.rotateScim.mockRejectedValueOnce(refused());
    await user.click(screen.getByRole("button", { name: "Make a new token" }));
    await user.click(screen.getByRole("button", { name: "Make a new token" }));
    expect(await textOf(screen.findByRole("alert"))).toBe("the server said no");
  });
});
