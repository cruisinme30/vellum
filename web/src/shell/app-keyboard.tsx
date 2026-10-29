// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The app's keyboard: the ⌘K menu with instant page titles (R-SR-1), `Q`
 * for quick-add anywhere (R-MW-3), and the `G` chords for the screens the
 * app has. Mounted inside the router so commands can navigate.
 */

import { useMemo, type ReactNode } from "react";
import { useNavigate } from "react-router";

import { searchTitles } from "../data/search";
import { useWorkspace } from "../data/workspace-context";
import type { PageFinder } from "../keyboard/command-menu";
import { KeyboardRoot } from "../keyboard/keyboard-root";
import { workCommands, type Destination } from "../keyboard/work-commands";
import { useQuickAdd } from "../work/quick-add";

/** The `G` chords with a screen behind them. */
const ROUTES: Partial<Record<Destination, string>> = {
  inbox: "/inbox",
  "my-work": "/my-work",
};

/** How many pages the menu lists before the commands. */
const PAGE_LIMIT = 8;

/** Wraps the routes with the app's shortcuts and the ⌘K menu. */
export function AppKeyboard({ children }: { readonly children: ReactNode }) {
  const navigate = useNavigate();
  const { pages } = useWorkspace();
  const quickAdd = useQuickAdd();
  const commands = useMemo(
    () =>
      workCommands({
        ...(quickAdd !== undefined && {
          quickAddTask: () => {
            quickAdd();
          },
        }),
        destinations: Object.keys(ROUTES) as Destination[],
        goTo: (destination) => {
          const path = ROUTES[destination];
          if (path !== undefined) void navigate(path);
        },
      }),
    [quickAdd, navigate],
  );
  const finder = useMemo<PageFinder>(
    () => ({
      match: (query) => searchTitles(pages, query, PAGE_LIMIT),
      open: (id) => {
        void navigate(`/p/${encodeURIComponent(id)}`);
      },
      searchText: (query) => {
        void navigate(`/search?q=${encodeURIComponent(query)}`);
      },
    }),
    [pages, navigate],
  );
  return (
    <KeyboardRoot commands={commands} pages={finder}>
      {children}
    </KeyboardRoot>
  );
}
