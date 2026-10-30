// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../../i18n/i18n";

import type { UserView } from "./api";
import { SignInHelp } from "./sign-in-help";

const MARCO: UserView = {
  id: "u2",
  email: "marco@northstar.dev",
  name: "Marco Ruiz",
  handle: "marco",
  admin: false,
  disabled: false,
  hasPassword: true,
  secondFactor: true,
  createdAt: "2026-10-01T10:00:00Z",
};

describe("sign-in help on Admin → Users", () => {
  it("asks before resetting someone's second factors or signing them out", async () => {
    const onSignInHelp = vi.fn();
    const user = userEvent.setup();
    render(
      <I18nProvider>
        <SignInHelp user={MARCO} busy={false} onSignInHelp={onSignInHelp} />
      </I18nProvider>,
    );
    await user.click(
      screen.getByRole("button", { name: "Reset two-step verification: Marco Ruiz" }),
    );
    expect(onSignInHelp).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Reset two-step verification" }));
    expect(onSignInHelp).toHaveBeenLastCalledWith("reset-factors");
    await user.click(screen.getByRole("button", { name: "Sign out everywhere: Marco Ruiz" }));
    await user.click(screen.getByRole("button", { name: "Keep" }));
    await user.click(screen.getByRole("button", { name: "Sign out everywhere: Marco Ruiz" }));
    await user.click(screen.getByRole("button", { name: "Sign out everywhere" }));
    expect(onSignInHelp).toHaveBeenLastCalledWith("sign-out");
  });

  it("offers no reset without a second factor", () => {
    render(
      <I18nProvider>
        <SignInHelp user={{ ...MARCO, secondFactor: false }} busy={false} onSignInHelp={vi.fn()} />
      </I18nProvider>,
    );
    expect(screen.queryByRole("button", { name: /Reset/ })).toBeNull();
  });
});
