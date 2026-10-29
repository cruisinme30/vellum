// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { ApiError } from "../api/http";

/** A failure as one line for the person, with the request ID to quote. */
export function failureText(error: unknown): string {
  if (error instanceof ApiError) {
    return error.requestId === undefined
      ? error.message
      : `${error.message} (request ${error.requestId})`;
  }
  return error instanceof Error ? error.message : String(error);
}
