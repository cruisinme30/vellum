// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AuthApi, AuthStatus } from "../api/auth";
import { ApiError } from "../api/http";
import { AppProviders } from "../app";
import { createMockCollectionsStore } from "../collections/mock";
import { createNorthstarStore } from "../data/mock/northstar";
import { createTestCollab } from "../test/render-app";

import type { Answer, AssistApi, McpSetupApi } from "./api";
import { createHttpMcpSetupApi } from "./api";
import { Ask } from "./ask-view";
import { McpSetup } from "./mcp-setup-page";
import { PageAssistant } from "./page-assistant";
import { forgetModel, loadModel, saveModel, toSettings } from "./settings";
import { AssistantSettings } from "./settings-view";

const USER = {
  id: "01M6409GN2EYWS8Z7NFQQ3JNWZ",
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: false,
};
const STATUS: AuthStatus = { needsSetup: false, credential: "session", scopes: [], user: USER };
const AUTH: AuthApi = {
  status: () => Promise.resolve(STATUS),
  signIn: () => Promise.resolve(USER),
  signOut: () => Promise.resolve(),
  setUp: () => Promise.resolve(USER),
};

const ANSWER: Answer = {
  text: "We launch on **24 November** [1].",
  citations: [
    {
      n: 1,
      doc_id: "01JDLAUNCH",
      title: "Launch plan › Goals",
      snippet: "Ship the policy simulator on 2026-11-24.",
      url: `${globalThis.location.origin}/p/01JDLAUNCH#blk-ship0001`,
    },
  ],
  truncated: false,
  model: "claude-opus-5-5",
};

function fakeAssist(overrides: Partial<AssistApi> = {}) {
  const spies = {
    test: vi.fn<AssistApi["test"]>(() =>
      Promise.resolve({ ok: true, detail: "claude-opus-5-5 answered." }),
    ),
    summarize: vi.fn<AssistApi["summarize"]>(() => Promise.resolve({ ...ANSWER, citations: [] })),
    ask: vi.fn<AssistApi["ask"]>(() => Promise.resolve(ANSWER)),
    draft: vi.fn<AssistApi["draft"]>(() =>
      Promise.resolve({
        page_id: "01JDNEW",
        url: `${globalThis.location.origin}/p/01JDNEW`,
        markdown: "## Problem",
        truncated: false,
        model: "claude-opus-5-5",
      }),
    ),
  };
  const api: AssistApi = { ...spies, ...overrides };
  return { api, ...spies };
}

async function textOf(found: Promise<HTMLElement>): Promise<string> {
  const element = await found;
  return element.textContent;
}

const OK_REPORT = {
  ok: true,
  checks: [{ name: "token", ok: true, detail: "Signed in as Priya." }],
  scopes: ["read"],
};

function fakeSetup() {
  const snippets = vi.fn<McpSetupApi["snippets"]>(() =>
    Promise.resolve([
      {
        client: "claude-code",
        title: "Claude Code",
        instructions: "Run in a terminal.",
        language: "shell" as const,
        code: "claude mcp add --transport http vellum https://vellum.northstar.dev/mcp",
      },
    ]),
  );
  const testConnection = vi.fn<McpSetupApi["testConnection"]>(() => Promise.resolve(OK_REPORT));
  return { api: { snippets, testConnection }, testConnection };
}

function Probe({ path }: { readonly path: string }) {
  return <p data-testid="at">{path}</p>;
}

function renderSignedIn(element: ReactNode, path = "/") {
  return render(
    <AppProviders
      store={createNorthstarStore()}
      collab={createTestCollab()}
      collections={createMockCollectionsStore()}
      session={{ auth: AUTH, status: STATUS }}
    >
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/p/:id" element={<Probe path="page" />} />
          <Route path="*" element={element} />
        </Routes>
      </MemoryRouter>
    </AppProviders>,
  );
}

const SAVED = {
  provider: "anthropic" as const,
  baseUrl: "",
  model: "claude-opus-5-5",
  apiKey: "sk-ant-secret-1234",
  remember: false,
};

afterEach(() => {
  forgetModel(USER.id);
  localStorage.clear();
  sessionStorage.clear();
});

describe("model settings in the browser", () => {
  it("keeps the key in session storage unless remembered, and never sends empty fields", () => {
    saveModel(USER.id, SAVED);
    expect(sessionStorage.getItem(`vellum.assist.${USER.id}`)).toContain("sk-ant-secret-1234");
    expect(localStorage.getItem(`vellum.assist.${USER.id}`)).toBeNull();
    saveModel(USER.id, { ...SAVED, remember: true });
    expect(sessionStorage.getItem(`vellum.assist.${USER.id}`)).toBeNull();
    expect(loadModel(USER.id)?.remember).toBe(true);
    expect(toSettings({ ...SAVED, apiKey: " " })).toEqual({
      provider: "anthropic",
      model: "claude-opus-5-5",
    });
    forgetModel(USER.id);
    expect(loadModel(USER.id)).toBeUndefined();
  });

  it("tests the model before saving it", async () => {
    const user = userEvent.setup();
    const { api, test } = fakeAssist();
    renderSignedIn(<AssistantSettings api={api} />);
    await user.type(screen.getByLabelText("API key"), "sk-ant-new-9999");
    await user.click(screen.getByRole("button", { name: "Test and save" }));
    expect(test).toHaveBeenCalledWith({
      provider: "anthropic",
      model: "claude-opus-5-5",
      api_key: "sk-ant-new-9999",
    });
    expect(await textOf(screen.findByRole("status"))).toContain("Saved in this browser.");
    expect(loadModel(USER.id)?.apiKey).toBe("sk-ant-new-9999");
  });

  it("does not save a model that fails the test", async () => {
    const user = userEvent.setup();
    const { api } = fakeAssist({
      test: vi.fn(() =>
        Promise.resolve({ ok: false, detail: "The provider refused the key (invalid x-api-key)." }),
      ),
    });
    renderSignedIn(<AssistantSettings api={api} />);
    await user.type(screen.getByLabelText("API key"), "bad");
    await user.click(screen.getByRole("button", { name: "Test and save" }));
    expect(await textOf(screen.findByRole("alert"))).toContain("refused the key");
    expect(loadModel(USER.id)).toBeUndefined();
  });
});

describe("ask this space", () => {
  it("asks for a model first", () => {
    renderSignedIn(<Ask api={fakeAssist().api} />);
    expect(screen.getByRole("link", { name: "Set up a model" }).getAttribute("href")).toBe(
      "/settings/ai",
    );
  });

  it("answers with links to the cited blocks", async () => {
    saveModel(USER.id, SAVED);
    const user = userEvent.setup();
    const { api, ask } = fakeAssist();
    renderSignedIn(<Ask api={api} />, "/ask?q=When%20do%20we%20launch%3F");
    expect(screen.getByLabelText("Question")).toHaveProperty("value", "When do we launch?");
    await user.click(screen.getByRole("button", { name: "Ask" }));
    expect(ask).toHaveBeenCalledWith(
      { provider: "anthropic", model: "claude-opus-5-5", api_key: "sk-ant-secret-1234" },
      "When do we launch?",
      undefined,
    );
    const sources = await screen.findByRole("region", { name: "Sources" });
    const link = within(sources).getByRole("link", { name: "Launch plan › Goals" });
    expect(link.getAttribute("href")).toBe("/p/01JDLAUNCH#blk-ship0001");
    expect(screen.getByText("24 November").tagName).toBe("STRONG");
  });

  it("shows the provider's error", async () => {
    saveModel(USER.id, SAVED);
    const user = userEvent.setup();
    const { api } = fakeAssist({
      ask: vi.fn(() =>
        Promise.reject(new ApiError("rate_limited", "the provider is rate limiting this key")),
      ),
    });
    renderSignedIn(<Ask api={api} />);
    await user.type(screen.getByLabelText("Question"), "Status?");
    await user.click(screen.getByRole("button", { name: "Ask" }));
    expect(await textOf(screen.findByRole("alert"))).toContain("rate limiting");
  });
});

describe("page assistant", () => {
  it("summarises the page and drafts a new page from it", async () => {
    saveModel(USER.id, SAVED);
    const user = userEvent.setup();
    const { api, summarize, draft } = fakeAssist();
    renderSignedIn(<PageAssistant api={api} pageId="01JDLAUNCH" onClose={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Summarise this page" }));
    expect(summarize).toHaveBeenCalledWith(
      expect.objectContaining({ model: "claude-opus-5-5" }),
      "01JDLAUNCH",
    );
    expect(await screen.findByText(/Written by claude-opus-5-5/)).toBeTruthy();
    await user.type(screen.getByLabelText("Title of the new page"), "Billing launch");
    await user.type(screen.getByLabelText("What it should say"), "Ship billing in December.");
    await user.click(screen.getByRole("button", { name: "Draft the page" }));
    expect(draft).toHaveBeenCalledWith(expect.anything(), {
      templatePageId: "01JDLAUNCH",
      title: "Billing launch",
      instructions: "Ship billing in December.",
    });
    expect(await textOf(screen.findByTestId("at"))).toBe("page");
  });
});

describe("MCP setup page", () => {
  it("lists the snippets and tests a token", async () => {
    const user = userEvent.setup();
    const { api, testConnection } = fakeSetup();
    renderSignedIn(<McpSetup api={api} />);
    expect(await screen.findByText(/claude mcp add/)).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Claude Code" })).toBeTruthy();
    await user.type(screen.getByLabelText("Token"), "vlm_token");
    await user.click(screen.getByRole("button", { name: "Test connection" }));
    expect(testConnection).toHaveBeenCalledWith("vlm_token");
    const status = await screen.findByText("The connection works.");
    expect(status).toBeTruthy();
    expect(screen.getByText(/The token can use: read/)).toBeTruthy();
  });

  it("sends the token only as a bearer header, without cookies", async () => {
    const doFetch = vi.fn<typeof fetch>(() =>
      Promise.resolve(Response.json({ ok: false, checks: [], scopes: [] })),
    );
    await createHttpMcpSetupApi(doFetch).testConnection(" vlm_x ");
    const [url, init] = doFetch.mock.calls[0] ?? [];
    expect(url).toBe("/mcp/test-connection");
    expect(init?.credentials).toBe("omit");
    expect(init?.headers).toEqual({ authorization: "Bearer vlm_x" });
  });
});
