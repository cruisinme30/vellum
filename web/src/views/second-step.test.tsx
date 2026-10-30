// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SecondStepRequired, type SignInSteps } from "../api/auth";
import { ApiError } from "../api/http";
import { I18nProvider } from "../i18n/i18n";

import { SecondStep } from "./second-step";

const passkeys = vi.hoisted(() => ({
  supported: true,
  create: vi.fn(),
  sign: vi.fn(),
}));

vi.mock("../account/webauthn", () => ({
  passkeysSupported: () => passkeys.supported,
  createPasskey: passkeys.create,
  signWithPasskey: passkeys.sign,
}));

const USER = {
  id: "u1",
  email: "priya@northstar.dev",
  name: "Priya",
  handle: "priya",
  admin: false,
};

function steps() {
  const verify = vi.fn<SignInSteps["verify"]>(() => Promise.resolve(USER));
  const enrol = vi.fn<SignInSteps["enrol"]>(() =>
    Promise.resolve({ user: USER, recoveryCodes: ["aaaaa-bbbbb"] }),
  );
  const api: SignInSteps = {
    verify,
    passkeyOptions: vi.fn<SignInSteps["passkeyOptions"]>(() => Promise.resolve({ challenge: "x" })),
    authenticator: vi.fn<SignInSteps["authenticator"]>(() =>
      Promise.reject(new ApiError("unavailable", "try later")),
    ),
    enrol,
  };
  return { api, verify, enrol };
}

async function textOf(found: Promise<HTMLElement>): Promise<string | null> {
  const element = await found;
  return element.textContent;
}

function show(step: SecondStepRequired, api: SignInSteps) {
  const onSignedIn = vi.fn(() => Promise.resolve());
  const onStartOver = vi.fn();
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <SecondStep step={step} steps={api} onSignedIn={onSignedIn} onStartOver={onStartOver} />
    </I18nProvider>,
  );
  return { user, onSignedIn, onStartOver };
}

beforeEach(() => {
  passkeys.supported = true;
  passkeys.create.mockReset();
  passkeys.sign.mockReset();
});

describe("verifying the second step", () => {
  it("signs in with a passkey", async () => {
    passkeys.sign.mockResolvedValue({ id: "k" });
    const { api, verify } = steps();
    const { user, onSignedIn } = show(new SecondStepRequired("verify", "vmt_1", ["passkey"]), api);
    expect(screen.queryByRole("button", { name: "Verify" })).toBeNull();
    await user.click(screen.getByRole("button", { name: /passkey/i }));
    await waitFor(() => {
      expect(onSignedIn).toHaveBeenCalled();
    });
    expect(verify).toHaveBeenCalledWith("vmt_1", { passkey: { id: "k" } });
  });

  it("says so when the browser cancels the passkey", async () => {
    passkeys.sign.mockRejectedValue(new DOMException("cancelled", "NotAllowedError"));
    const { user } = show(new SecondStepRequired("verify", "vmt_1", ["passkey"]), steps().api);
    await user.click(screen.getByRole("button", { name: /passkey/i }));
    expect(await textOf(screen.findByRole("alert"))).not.toBe("cancelled");
  });

  it("asks for a recovery code when nothing else can work here", async () => {
    passkeys.supported = false;
    const { api, verify } = steps();
    const { user, onStartOver } = show(
      new SecondStepRequired("verify", "vmt_1", ["passkey", "recovery"]),
      api,
    );
    await user.type(screen.getByLabelText("Recovery code"), "aaaaa-bbbbb");
    await user.click(screen.getByRole("button", { name: "Verify" }));
    await waitFor(() => {
      expect(verify).toHaveBeenCalledWith("vmt_1", { code: "aaaaa-bbbbb" });
    });
    await user.click(screen.getByRole("button", { name: /start over/i }));
    expect(onStartOver).toHaveBeenCalled();
  });
});

describe("enrolling during sign-in", () => {
  it("adds a passkey, shows the recovery codes, then signs in", async () => {
    passkeys.create.mockResolvedValue({ id: "new" });
    const { api, enrol } = steps();
    const { user, onSignedIn } = show(
      new SecondStepRequired("enrol", "vmt_e", ["totp", "passkey"]),
      api,
    );
    await user.type(screen.getByRole("textbox"), "Laptop");
    await user.click(screen.getByRole("button", { name: /passkey/i }));
    expect(await textOf(screen.findByTestId("recovery-codes"))).toContain("aaaaa-bbbbb");
    expect(enrol).toHaveBeenCalledWith("vmt_e", {
      passkey: { name: "Laptop", credential: { id: "new" } },
    });
    await user.click(screen.getByRole("button", { name: "I have saved them" }));
    await waitFor(() => {
      expect(onSignedIn).toHaveBeenCalled();
    });
  });

  it("shows why an app could not be set up, and hides passkeys the browser lacks", async () => {
    passkeys.supported = false;
    const { user } = show(new SecondStepRequired("enrol", "vmt_e", ["totp"]), steps().api);
    expect(screen.queryByRole("textbox")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Set up an authenticator app" }));
    expect(await textOf(screen.findByRole("alert"))).toBe("try later");
  });
});
