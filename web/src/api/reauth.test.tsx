// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "../i18n/i18n";

import type { AuthApi, AuthStatus } from "./auth";
import { ApiError, type Http } from "./http";
import {
  createReauthGate,
  needsReauth,
  REAUTHENTICATION_REQUIRED,
  ssoReauthUrl,
  withReauth,
} from "./reauth";
import { ReauthPrompt } from "./reauth-prompt";
import { SessionProvider } from "./session";

const stale = () => new ApiError("unauthenticated", REAUTHENTICATION_REQUIRED, { status: 401 });

function fakeHttp(answers: (() => Promise<unknown>)[]) {
  const send = vi.fn(() => {
    const next = answers.shift();
    return next === undefined ? Promise.resolve() : next();
  });
  const http = { get: send, send, forgetSession: vi.fn() } as unknown as Http;
  return { http, send };
}

describe("withReauth", () => {
  it("asks once and runs the request again", async () => {
    const gate = createReauthGate();
    const { http, send } = fakeHttp([() => Promise.reject(stale()), () => Promise.resolve("ok")]);
    const call = withReauth(http, gate).send("POST", "/api/v1/admin/users", {});
    await waitFor(() => {
      expect(gate.pending()).toBe(true);
    });
    gate.settle(true);
    await expect(call).resolves.toBe("ok");
    expect(send).toHaveBeenCalledTimes(2);
  });

  it("fails with the server's error when the person cancels", async () => {
    const gate = createReauthGate();
    const { http } = fakeHttp([() => Promise.reject(stale())]);
    const call = withReauth(http, gate).get("/api/v1/admin/users");
    await waitFor(() => {
      expect(gate.pending()).toBe(true);
    });
    gate.settle(false);
    await expect(call).rejects.toSatisfy(needsReauth);
  });

  it("shares one prompt between waiting requests and leaves other errors alone", async () => {
    const gate = createReauthGate();
    const request = vi.spyOn(gate, "request");
    expect(gate.request()).toBe(gate.request());
    gate.settle(true);
    request.mockClear();
    const { http } = fakeHttp([
      () => Promise.reject(new ApiError("unauthenticated", "sign in to continue")),
    ]);
    await expect(withReauth(http, gate).get("/x")).rejects.toThrow("sign in to continue");
    expect(request).not.toHaveBeenCalled();
  });

  it("sends single sign-on back to the page", () => {
    expect(ssoReauthUrl("/auth/sso/start", "/admin/users")).toBe(
      "/auth/sso/start?reauth=1&next=%2Fadmin%2Fusers",
    );
  });
});

function renderPrompt(status: Partial<AuthStatus>, http: Http) {
  const gate = createReauthGate();
  const auth = { status: vi.fn() } as unknown as AuthApi;
  const full: AuthStatus = { needsSetup: false, credential: "session", scopes: [], ...status };
  render(
    <I18nProvider>
      <SessionProvider auth={auth} initial={full}>
        <ReauthPrompt gate={gate} http={http} />
      </SessionProvider>
    </I18nProvider>,
  );
  return gate;
}

const ADMIN = {
  id: "u1",
  email: "priya@northstar.dev",
  name: "Priya",
  handle: "priya",
  admin: true,
};

describe("the re-authentication dialog", () => {
  it("takes the password again and lets the action run", async () => {
    const user = userEvent.setup();
    const { http, send } = fakeHttp([
      () => Promise.reject(new ApiError("unauthenticated", "wrong password", { status: 401 })),
      () => Promise.resolve(),
    ]);
    const gate = renderPrompt({ user: { ...ADMIN, hasPassword: true } }, http);
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = gate.request();
    });
    const dialog = await screen.findByRole("dialog", { name: "Confirm it's you" });
    expect(dialog).toBeDefined();
    const password = screen.getByLabelText("Password");
    expect(document.activeElement).toBe(password);

    await user.type(password, "not it");
    await user.click(screen.getByRole("button", { name: "Confirm" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("wrong password");

    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Confirm" }));
    await expect(answer).resolves.toBe(true);
    expect(send).toHaveBeenLastCalledWith("POST", "/auth/reauthenticate", {
      password: "correct horse battery",
    });
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("offers single sign-on to an account without a password, and Escape cancels", async () => {
    const user = userEvent.setup();
    const { http } = fakeHttp([]);
    const gate = renderPrompt(
      {
        user: { ...ADMIN, hasPassword: false },
        sso: { name: "Northstar SSO", startUrl: "/auth/sso/start" },
      },
      http,
    );
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = gate.request();
    });
    const link = await screen.findByRole("link", { name: "Continue with Northstar SSO" });
    expect(link.getAttribute("href")).toContain("reauth=1");
    expect(screen.queryByLabelText("Password")).toBeNull();
    await user.keyboard("{Escape}");
    await expect(answer).resolves.toBe(false);
  });
});
