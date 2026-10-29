// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AuthApi, AuthStatus, User } from "../api/auth";
import { ApiError } from "../api/http";
import { safeNext } from "../api/session";
import { renderApp } from "../test/render-app";

import { describeError } from "./sign-in";

const USER: User = {
  id: "u1",
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: true,
};
const SIGNED_OUT: AuthStatus = {
  needsSetup: false,
  credential: "anonymous",
  scopes: [],
};
const SIGNED_IN: AuthStatus = {
  needsSetup: false,
  credential: "session",
  scopes: ["read"],
  user: USER,
};
const NEW: AuthStatus = { needsSetup: true, credential: "anonymous", scopes: [] };

/** A fake server whose status moves on as people sign in, out or set up. */
function fakeAuth(start: AuthStatus) {
  let status = start;
  const signIn = vi.fn((email: string) => {
    if (email === "wrong@northstar.dev") {
      return Promise.reject(
        new ApiError("unauthenticated", "Wrong email or password.", { requestId: "01JREQ" }),
      );
    }
    status = SIGNED_IN;
    return Promise.resolve(USER);
  });
  const signOut = vi.fn(() => {
    status = SIGNED_OUT;
    return Promise.resolve();
  });
  const setUp = vi.fn(() => {
    status = SIGNED_IN;
    return Promise.resolve(USER);
  });
  const auth: AuthApi = { status: () => Promise.resolve(status), signIn, signOut, setUp };
  return { auth, status: start, signIn, signOut, setUp };
}

describe("signing in against a server", () => {
  it("sends a signed-out visitor to sign-in, then back where they were", async () => {
    const session = fakeAuth(SIGNED_OUT);
    const app = renderApp("/data?x=1", undefined, undefined, session);
    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeTruthy();
    expect(app.location()).toBe(`/sign-in?next=${encodeURIComponent("/data?x=1")}`);
    await app.user.type(screen.getByLabelText("Email"), "priya@northstar.dev");
    await app.user.type(screen.getByLabelText("Password"), "correct horse");
    await app.user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => {
      expect(app.location()).toBe("/data?x=1");
    });
    expect(session.signIn).toHaveBeenCalledWith("priya@northstar.dev", "correct horse");
  });

  it("goes to sign-in without a next for the home page", async () => {
    const app = renderApp("/", undefined, undefined, fakeAuth(SIGNED_OUT));
    await screen.findByRole("heading", { name: "Sign in" });
    expect(app.location()).toBe("/sign-in");
  });

  it("shows a refused sign-in with its request ID", async () => {
    const app = renderApp("/sign-in", undefined, undefined, fakeAuth(SIGNED_OUT));
    await app.user.type(await screen.findByLabelText("Email"), "wrong@northstar.dev");
    await app.user.type(screen.getByLabelText("Password"), "nope nope nope");
    await app.user.click(screen.getByRole("button", { name: "Sign in" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Wrong email or password. (request 01JREQ)");
    expect(app.location()).toBe("/sign-in");
  });

  it("hands an app's consent page back to the server", async () => {
    const assign = vi.fn();
    vi.stubGlobal("location", { href: globalThis.location.href, assign });
    const app = renderApp(
      "/sign-in?next=%2Foauth%2Fauthorize%3Fx%3D1",
      undefined,
      undefined,
      fakeAuth(SIGNED_OUT),
    );
    await app.user.type(await screen.findByLabelText("Email"), "priya@northstar.dev");
    await app.user.type(screen.getByLabelText("Password"), "correct horse");
    await app.user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => {
      expect(assign).toHaveBeenCalledWith("/oauth/authorize?x=1");
    });
    vi.unstubAllGlobals();
  });

  it("lets a signed-in user straight in and greets them by name", async () => {
    renderApp("/", undefined, undefined, fakeAuth(SIGNED_IN));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Good to see you, Priya" }),
    ).toBeTruthy();
  });

  it("signs out from the admin page", async () => {
    const session = fakeAuth(SIGNED_IN);
    const app = renderApp("/admin", undefined, undefined, session);
    expect(
      await screen.findByRole("heading", {
        name: "Signed in as Priya Raman (priya@northstar.dev)",
      }),
    ).toBeTruthy();
    await app.user.click(screen.getByRole("button", { name: "Sign out" }));
    await screen.findByRole("heading", { level: 1, name: "Sign in" });
    expect(session.signOut).toHaveBeenCalled();
  });

  it("shows a failed sign-out", async () => {
    const session = fakeAuth(SIGNED_IN);
    session.signOut.mockRejectedValueOnce(new Error("socket closed"));
    const app = renderApp("/admin", undefined, undefined, session);
    await app.user.click(await screen.findByRole("button", { name: "Sign out" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Something went wrong. Try again.");
  });
});

describe("setting up a new workspace", () => {
  it("sends everyone to set-up and creates the first administrator", async () => {
    const session = fakeAuth(NEW);
    const app = renderApp("/search", undefined, undefined, session);
    expect(await screen.findByRole("heading", { name: "Set up your workspace" })).toBeTruthy();
    await app.user.type(screen.getByLabelText("Your name"), "Priya Raman");
    await app.user.type(screen.getByLabelText("Email"), "priya@northstar.dev");
    const password = screen.getByLabelText("Password");
    expect(password.getAttribute("aria-describedby")).toBeTruthy();
    await app.user.type(password, "a long enough password");
    await app.user.click(screen.getByRole("button", { name: "Create account" }));
    await waitFor(() => {
      expect(app.location()).toBe("/");
    });
    expect(session.setUp).toHaveBeenCalledWith({
      name: "Priya Raman",
      email: "priya@northstar.dev",
      password: "a long enough password",
    });
  });

  it("sends sign-in to set-up while the workspace is new", async () => {
    const app = renderApp("/sign-in", undefined, undefined, fakeAuth(NEW));
    await screen.findByRole("heading", { name: "Set up your workspace" });
    expect(app.location()).toBe("/setup");
  });

  it("leaves set-up once the workspace exists", async () => {
    const app = renderApp("/setup", undefined, undefined, fakeAuth(SIGNED_IN));
    await screen.findByRole("heading", { name: "Good to see you, Priya" });
    expect(app.location()).toBe("/");
  });
});

describe("in demo mode", () => {
  it("has no sign-in, set-up or account section", async () => {
    const signIn = renderApp("/sign-in");
    await waitFor(() => {
      expect(signIn.location()).toBe("/");
    });
    signIn.unmount();
    const setup = renderApp("/setup");
    await waitFor(() => {
      expect(setup.location()).toBe("/");
    });
    setup.unmount();
    renderApp("/admin");
    await screen.findByRole("heading", { level: 1, name: "Admin" });
    expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
  });
});

describe("helpers", () => {
  it("keeps the next address inside the app", () => {
    expect(safeNext(new URLSearchParams().get("next"))).toBe("/");
    expect(safeNext("/data")).toBe("/data");
    expect(safeNext("https://evil.example")).toBe("/");
    expect(safeNext("//evil.example")).toBe("/");
    expect(safeNext(String.raw`/\evil.example`)).toBe("/");
  });

  it("describes errors", () => {
    expect(describeError(new ApiError("conflict", "Changed"), "x")).toBe("Changed");
    expect(describeError(new Error("raw"), "fallback")).toBe("fallback");
  });
});
