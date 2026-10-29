// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef } from "react";

import type { Command, Scope } from "./keymap";
import { formatBinding, parseBinding, type KeyPlatform } from "./keys";

/** Props of the shortcut sheet. */
export interface ShortcutSheetProperties {
  readonly commands: readonly Command[];
  readonly platform: KeyPlatform;
  /** Scopes in force on this screen; the sheet shows only those. */
  readonly scopes: ReadonlySet<Scope>;
  readonly onClose: () => void;
}

const HEADINGS: Readonly<Record<Scope, string>> = {
  global: "Everywhere",
  issues: "In issue lists",
  editor: "In the editor",
};

const ORDER: readonly Scope[] = ["global", "issues", "editor"];

/** The `?` sheet: every shortcut for the current screen, by where it applies. */
export function ShortcutSheet({ commands, platform, scopes, onClose }: ShortcutSheetProperties) {
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    close.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose]);

  return (
    <div
      className="shortcut-sheet"
      role="dialog"
      aria-modal="true"
      aria-labelledby="shortcut-sheet-title"
    >
      <h2 id="shortcut-sheet-title">Keyboard shortcuts</h2>
      {ORDER.filter((scope) => scopes.has(scope)).flatMap((scope) => {
        const rows = commands.filter((c) => c.scope === scope && c.keys.length > 0);
        if (rows.length === 0) {
          return [];
        }
        return [
          <section key={scope} aria-label={HEADINGS[scope]}>
            <h3>{HEADINGS[scope]}</h3>
            <table>
              <tbody>
                {rows.map((command) => (
                  <tr key={command.id}>
                    <td>
                      {command.keys.map((k) => (
                        <kbd key={k}>{formatBinding(parseBinding(k), platform)}</kbd>
                      ))}
                    </td>
                    <td>{command.title}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>,
        ];
      })}
      <button ref={close} type="button" onClick={onClose}>
        Close
      </button>
    </div>
  );
}
