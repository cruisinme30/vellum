// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { KeyboardRoot } from "./keyboard/keyboard-root";

/**
 * The application shell. The editor, drive and work views mount here as
 * their workstreams land.
 */
export function App() {
  return (
    <KeyboardRoot>
      <main>
        <h1>Vellum</h1>
        <p>A self-hosted workspace where every object is a Markdown file.</p>
      </main>
    </KeyboardRoot>
  );
}
