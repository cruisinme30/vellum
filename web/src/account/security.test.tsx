// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import type { AccountApi, FactorsView } from "./api";
import { SecurityPage } from "./security-page";
import { fromBase64Url, toBase64Url } from "./webauthn";

const NONE: FactorsView = { totp: false, passkeys: [], recoveryCodesLeft: 0, required: false };
const unused = () => Promise.reject(new Error("not in this test"));

function fakeAccount(initial: FactorsView) {
  const confirmTotp = vi.fn<AccountApi["confirmTotp"]>((code) =>
    code === "123456"
      ? Promise.resolve({
          factors: { ...initial, totp: true, recoveryCodesLeft: 10 },
          recoveryCodes: ["aaaaa-bbbbb", "ccccc-ddddd"],
        })
      : Promise.reject(new Error("the code is wrong")),
  );
  const removeTotp = vi.fn<AccountApi["removeTotp"]>(() => Promise.resolve(NONE));
  const api: AccountApi = {
    tokens: unused,
    tokenPolicy: unused,
    createToken: unused,
    revokeToken: unused,
    factors: () => Promise.resolve(initial),
    startTotp: () =>
      Promise.resolve({ secret: "JBSWY3DPEHPK3PXP", uri: "otpauth://x", qr: ["11", "01"] }),
    confirmTotp,
    removeTotp,
    startPasskey: unused,
    addPasskey: unused,
    removePasskey: unused,
    newRecoveryCodes: () => Promise.resolve(["eeeee-fffff"]),
  };
  return { api, confirmTotp, removeTotp };
}

function renderSecurity(api: AccountApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ account: api }}>
        <SecurityPage />
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

describe("base64url", () => {
  it("round-trips bytes without padding", () => {
    const bytes = new Uint8Array([0, 251, 255, 16, 1]);
    const text = toBase64Url(bytes);
    expect(text).not.toMatch(/[+/=]/);
    expect([...fromBase64Url(text)]).toEqual([...bytes]);
    expect([...fromBase64Url(`${text}=`)]).toEqual([...bytes]);
  });
});

describe("Settings → Two-step verification", () => {
  it("adds an authenticator app and shows the recovery codes once", async () => {
    const { api, confirmTotp } = fakeAccount(NONE);
    const user = renderSecurity(api);
    await user.click(await screen.findByRole("button", { name: "Set up an authenticator app" }));
    expect(await screen.findByRole("img", { name: /QR code/ })).toBeDefined();
    const code = screen.getByLabelText("Enter the 6-digit code it shows");
    await user.type(code, "000000");
    await user.click(screen.getByRole("button", { name: "Turn on" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("the code is wrong");
    await user.clear(code);
    await user.type(code, "123456");
    await user.click(screen.getByRole("button", { name: "Turn on" }));
    expect(confirmTotp).toHaveBeenLastCalledWith("123456");
    const codes = await screen.findByTestId("recovery-codes");
    expect(codes.textContent).toContain("ccccc-ddddd");
    await user.click(screen.getByRole("button", { name: "I have saved them" }));
    expect(screen.queryByTestId("recovery-codes")).toBeNull();
    expect(
      screen.getByText(
        "10 recovery codes left. Each works once, if you lose your app and passkeys.",
      ),
    ).toBeDefined();
  });

  it("removes the app after a confirmation and says when the workspace requires one", async () => {
    const { api, removeTotp } = fakeAccount({
      ...NONE,
      totp: true,
      recoveryCodesLeft: 4,
      required: true,
    });
    const user = renderSecurity(api);
    expect(await screen.findByText(/requires two-step verification/)).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Remove app" }));
    expect(removeTotp).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect(removeTotp).toHaveBeenCalled();
    expect(await screen.findByText("Authenticator app removed.")).toBeDefined();
  });
});
