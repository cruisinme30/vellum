// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The `fetch` shape both Node's global and Electron's `net.fetch` satisfy. */
export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;
