// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { RequestsPage } from "./admin/requests/requests-page";
import { RoutesPage } from "./admin/routes/routes-page";
import { KeyboardRoot } from "./keyboard/keyboard-root";

function Page({ path }: { readonly path: string }) {
  if (path.startsWith("/admin/requests")) {
    return <RequestsPage />;
  }
  if (path.startsWith("/admin/routes")) {
    return <RoutesPage />;
  }
  return (
    <main>
      <h1>Vellum</h1>
      <p>A self-hosted workspace where every object is a Markdown file.</p>
    </main>
  );
}

/**
 * The application shell. The editor, drive and work views mount here as
 * their workstreams land.
 */
export function App({ path = globalThis.location.pathname }: { readonly path?: string }) {
  return (
    <KeyboardRoot>
      <Page path={path} />
    </KeyboardRoot>
  );
}
