// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { telemetry } from "../platform/telemetry";

import "./keyboard.css";

import { CommandMenu, type PageFinder } from "./command-menu";
import { Keymap, type Command, type Scope } from "./keymap";
import { currentPlatform, type KeyPlatform } from "./keys";
import { ShortcutSheet } from "./shortcut-sheet";

/** Props of the keyboard root. */
export interface KeyboardRootProperties {
  /** The screen's commands, such as `workCommands(...)`. */
  readonly commands?: readonly Command[];
  /** Scopes in force besides `global`, such as `issues` on an issue list. */
  readonly scopes?: readonly Scope[];
  readonly platform?: KeyPlatform;
  /** Page search in the ⌘K menu; commands only when absent. */
  readonly pages?: PageFinder | undefined;
  readonly children?: ReactNode;
}

type Open = "none" | "menu" | "sheet";

const NO_COMMANDS: readonly Command[] = [];
const NO_SCOPES: readonly Scope[] = [];

/**
 * Listens for shortcuts on the whole document and owns the ⌘K command
 * menu and the `?` shortcut sheet. Mount it once around the app.
 */
export function KeyboardRoot({
  commands = NO_COMMANDS,
  scopes = NO_SCOPES,
  platform = currentPlatform(),
  pages,
  children,
}: KeyboardRootProperties) {
  const [open, setOpen] = useState<Open>("none");
  const returnFocus = useRef<Element | null>(null);
  const active = useMemo(() => new Set<Scope>(["global", ...scopes]), [scopes]);

  const all = useMemo<Command[]>(
    () => [
      {
        id: "menu.open",
        title: "Search pages and run any command",
        scope: "global",
        keys: ["mod+k"],
        run: () => {
          setOpen("menu");
        },
      },
      {
        id: "shortcuts.show",
        title: "Show all shortcuts",
        scope: "global",
        keys: ["?"],
        keywords: ["help", "keyboard"],
        run: () => {
          setOpen("sheet");
        },
      },
      ...commands,
    ],
    [commands],
  );
  const keymap = useMemo(() => new Keymap(all, { platform }), [all, platform]);

  useEffect(() => {
    if (open !== "none") {
      return;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) {
        return;
      }
      const started = performance.now();
      const result = keymap.dispatch(event, active);
      if (result.kind === "none") {
        return;
      }
      event.preventDefault();
      if (result.kind === "run") {
        returnFocus.current = document.activeElement;
        result.command.run();
        telemetry().histogram("ui.keyboard.dispatch", performance.now() - started, {
          command: result.command.id,
        });
        telemetry().count("ui.keyboard.commands", { command: result.command.id, via: "key" });
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [keymap, active, open]);

  const close = () => {
    setOpen("none");
    if (returnFocus.current instanceof HTMLElement) {
      returnFocus.current.focus();
    }
  };

  return (
    <>
      {children}
      {open === "menu" ? (
        <CommandMenu
          commands={all.filter((c) => active.has(c.scope) && c.id !== "menu.open")}
          pages={pages}
          platform={platform}
          onClose={close}
          onRun={(command) => {
            command.run();
            telemetry().count("ui.keyboard.commands", { command: command.id, via: "menu" });
          }}
        />
      ) : undefined}
      {open === "sheet" ? (
        <ShortcutSheet commands={all} platform={platform} scopes={active} onClose={close} />
      ) : undefined}
    </>
  );
}
