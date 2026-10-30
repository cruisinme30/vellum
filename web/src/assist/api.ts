// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bring-your-own-model features (`/api/v1/assist/*`, ADR 0056) and the MCP
 * setup routes (`/mcp/setup`, `/mcp/test-connection`). Vellum runs no
 * model: every call carries the person's own model settings and key.
 */

import type { Answer, Draft, ModelSettings, TestResult } from "../api/api.gen";
import { ApiError, type Http } from "../api/http";

/** What the assistant can do. */
export interface AssistApi {
  test(model: ModelSettings): Promise<TestResult>;
  summarize(model: ModelSettings, pageId: string): Promise<Answer>;
  ask(model: ModelSettings, question: string, space?: string): Promise<Answer>;
  draft(
    model: ModelSettings,
    input: {
      templatePageId: string;
      title: string;
      instructions: string;
      parent?: string;
    },
  ): Promise<Draft>;
}

/** The assistant over a Vellum server. */
export function createHttpAssistApi(http: Http): AssistApi {
  return {
    test: (model) =>
      http.send<TestResult>("POST", "/api/v1/assist/test", { model }),
    summarize: (model, pageId) =>
      http.send<Answer>("POST", "/api/v1/assist/summarize", {
        model,
        page_id: pageId,
      }),
    ask: (model, question, space) =>
      http.send<Answer>("POST", "/api/v1/assist/ask", {
        model,
        question,
        ...(space === undefined ? {} : { space }),
      }),
    draft: (model, input) =>
      http.send<Draft>("POST", "/api/v1/assist/draft", {
        model,
        template_page_id: input.templatePageId,
        title: input.title,
        instructions: input.instructions,
        ...(input.parent === undefined ? {} : { parent: input.parent }),
      }),
  };
}

/** One MCP client's setup, as `GET /mcp/setup` lists it. */
export interface Snippet {
  readonly client: string;
  readonly title: string;
  readonly instructions: string;
  readonly language: "json" | "shell";
  readonly code: string;
}

/** One step of "Test connection". */
interface ConnectionCheck {
  readonly name: string;
  readonly ok: boolean;
  readonly detail: string;
}

/** The result of "Test connection". */
export interface ConnectionReport {
  readonly ok: boolean;
  readonly checks: readonly ConnectionCheck[];
  readonly scopes: readonly string[];
}

/** The admin page's MCP routes. */
export interface McpSetupApi {
  snippets(): Promise<Snippet[]>;
  /**
   * Runs the check with `token`, sent once as a bearer token and never stored.
   */
  testConnection(token: string): Promise<ConnectionReport>;
}

async function readJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let message = `The server answered ${String(response.status)}.`;
    try {
      const body = (await response.json()) as { message?: unknown };
      if (typeof body.message === "string") message = body.message;
    } catch {
      // Not JSON.
    }
    throw new ApiError(
      response.status === 404 ? "not_found" : "unavailable",
      message,
      {
        status: response.status,
      },
    );
  }
  return (await response.json()) as T;
}

/** The MCP setup routes on the app's own origin. */
export function createHttpMcpSetupApi(
  doFetch: typeof fetch = (index, o) => fetch(index, o),
): McpSetupApi {
  return {
    async snippets() {
      const response = await doFetch("/mcp/setup", {
        credentials: "same-origin",
      });
      const body = await readJson<{ snippets: Snippet[] }>(response);
      return body.snippets;
    },
    async testConnection(token) {
      const response = await doFetch("/mcp/test-connection", {
        method: "POST",
        credentials: "omit",
        headers: { authorization: `Bearer ${token.trim()}` },
      });
      return readJson<ConnectionReport>(response);
    },
  };
}

export { type Answer, type TestResult } from "../api/api.gen";
