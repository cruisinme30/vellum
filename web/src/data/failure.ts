// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { ApiError } from "../api/http";

/** What support needs to find a failure in the server's logs (R-OB-1). */
export interface Diagnostic {
  readonly message: string;
  readonly requestId?: string | undefined;
  /** `METHOD /path`, without the query string. */
  readonly route?: string | undefined;
  readonly code?: string | undefined;
  readonly status?: number | undefined;
  readonly at: Date;
}

/** Failures shown recently, by the text the person sees; oldest dropped first. */
const RECENT = new Map<string, Diagnostic>();
const KEEP = 50;

/** The diagnostic behind an error, whatever threw it. */
export function diagnosticOf(error: unknown): Diagnostic {
  if (error instanceof ApiError) {
    return {
      message: error.message,
      requestId: error.requestId,
      route: error.route,
      code: error.code,
      status: error.status === 0 ? undefined : error.status,
      at: error.at,
    };
  }
  return { message: error instanceof Error ? error.message : String(error), at: new Date() };
}

/** A failure as one line for the person, with the request ID to quote. */
export function failureText(error: unknown): string {
  const diagnostic = diagnosticOf(error);
  const text =
    diagnostic.requestId === undefined
      ? diagnostic.message
      : `${diagnostic.message} (request ${diagnostic.requestId})`;
  RECENT.delete(text);
  RECENT.set(text, diagnostic);
  while (RECENT.size > KEEP) {
    const oldest = RECENT.keys().next().value;
    if (oldest === undefined) break;
    RECENT.delete(oldest);
  }
  return text;
}

/** The diagnostic for a failure line {@link failureText} produced, if it is recent. */
export function recentDiagnostic(text: string): Diagnostic | undefined {
  return RECENT.get(text);
}

/**
 * The text "Copy diagnostic" puts on the clipboard: the request ID, the
 * route and the time, plus where in the app it happened. It holds no page
 * content and no credentials, so it is safe to paste into a support ticket.
 */
export function diagnosticText(diagnostic: Diagnostic, where: string, app = "web"): string {
  const lines = [
    "Vellum diagnostic",
    `Message: ${diagnostic.message}`,
    `Request ID: ${diagnostic.requestId ?? "none (no answer from the server)"}`,
    `Route: ${diagnostic.route ?? "unknown"}`,
  ];
  if (diagnostic.code !== undefined || diagnostic.status !== undefined) {
    const status = diagnostic.status === undefined ? "" : ` (HTTP ${String(diagnostic.status)})`;
    lines.push(`Error: ${diagnostic.code ?? "unknown"}${status}`);
  }
  lines.push(`Time: ${diagnostic.at.toISOString()}`, `Screen: ${where}`, `Client: ${app}`);
  return lines.join("\n");
}
