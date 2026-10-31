// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

/** A loopback listener waiting for the OAuth redirect (RFC 8252 §7.3). */
export interface Redirect {
  /** `http://127.0.0.1:<port>/callback` */
  readonly uri: string;
  /**
   * The authorization code, once the browser comes back with a matching state.
   */
  readonly code: Promise<string>;
  close(): void;
}

const TIMEOUT_MS = 5 * 60_000;

const page = (message: string) =>
  `<!doctype html><meta charset="utf-8"><title>Vellum</title><body ` +
  `style="font:16px system-ui;margin:4rem ` +
  `auto;max-width:32rem">${message}</body>`;

/**
 * Listens on an ephemeral 127.0.0.1 port until the browser returns or five
 * minutes pass.
 */
export async function waitForRedirect(
  state: string,
  timeoutMs = TIMEOUT_MS,
): Promise<Redirect> {
  let settle:
    | { resolve: (code: string) => void; reject: (error: Error) => void }
    | undefined;
  const code = new Promise<string>((resolve, reject) => {
    settle = { resolve, reject };
  });
  const server: Server = createServer((request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (url.pathname !== "/callback") {
      response.writeHead(404).end();
      return;
    }
    const error = url.searchParams.get("error");
    const received = url.searchParams.get("code");
    const ok =
      error === null &&
      received !== null &&
      url.searchParams.get("state") === state;
    response
      .writeHead(ok ? 200 : 400, {
        "content-type": "text/html; charset=utf-8",
        "cache-control": "no-store",
      })
      .end(
        page(
          ok
            ? "You are signed in. You can close this tab and return to Vellum."
            : "Sign-in did not complete. Return to Vellum and try again.",
        ),
      );
    if (ok) settle?.resolve(received);
    else
      settle?.reject(
        new Error(error ?? "The sign-in response did not match this request"),
      );
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  const timer = setTimeout(
    () => settle?.reject(new Error("Sign-in timed out")),
    timeoutMs,
  );
  // Nothing waits on a rejected code once the caller has closed the redirect.
  code.catch(() => {
    // Handled by whoever awaits `code`.
  });
  return {
    uri: `http://127.0.0.1:${String(port)}/callback`,
    code,
    close() {
      clearTimeout(timer);
      server.close();
      settle?.reject(new Error("Sign-in was cancelled"));
    },
  };
}
