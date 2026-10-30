// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The Work keyboard model: the bindings in the user guide's shortcut page
 * (`docs/user.book/start/shortcuts.md`), wired to whatever the current
 * screen can do. A screen passes only the actions it supports; commands
 * for the rest are left out, so a key never does something half-wired.
 */

import type { Command, Scope } from "./keymap";

/** Where the `G` chords go. */
export type Destination = "inbox" | "my-work" | "board" | "cycle" | "triage";

/**
 * The `G` chords the app keyboard binds on every screen. A Work screen binds
 * only `SCREEN_DESTINATIONS`, because both sets are global and the keymap
 * throws when two commands share a chord.
 */
export const APP_DESTINATIONS: readonly Destination[] = ["inbox", "my-work"];

/** The `G` chords only the Work screens bind. */
export const SCREEN_DESTINATIONS: readonly Destination[] = ["board", "cycle", "triage"];

/** What the current screen can do. Every action is optional. */
export interface WorkActions {
  readonly newIssue?: () => void;
  readonly newIssueFromTemplate?: () => void;
  readonly quickAddTask?: () => void;
  readonly goTo?: (destination: Destination) => void;
  /** The destinations `goTo` can reach; all of them when absent. */
  readonly destinations?: readonly Destination[];
  /** Issue-list actions apply to the focused or selected issues. */
  readonly setStatus?: () => void;
  readonly setPriority?: () => void;
  readonly setLabels?: () => void;
  readonly assign?: () => void;
  readonly assignToMe?: () => void;
  readonly setEstimate?: () => void;
  readonly setCycle?: () => void;
  readonly toggleSelect?: () => void;
  readonly peek?: () => void;
  readonly copyBranchName?: () => void;
}

type Simple = Exclude<keyof WorkActions, "goTo" | "destinations">;

type Spec = readonly [Simple, string, string, Scope, (readonly string[])?];

const SPECS: readonly Spec[] = [
  ["newIssue", "issue.create", "New issue", "global", ["create", "ticket", "bug"]],
  [
    "newIssueFromTemplate",
    "issue.create-from-template",
    "New issue from a template",
    "global",
    ["template"],
  ],
  ["quickAddTask", "task.quick-add", "Quick-add a task", "global", ["todo"]],
  ["setStatus", "issue.set-status", "Set status", "issues", ["state", "move"]],
  ["setPriority", "issue.set-priority", "Set priority", "issues", ["urgent"]],
  ["setLabels", "issue.set-labels", "Set labels", "issues", ["tag"]],
  ["assign", "issue.assign", "Assign", "issues", ["owner"]],
  ["assignToMe", "issue.assign-to-me", "Assign to me", "issues", ["mine", "take"]],
  ["setEstimate", "issue.set-estimate", "Set estimate", "issues", ["points", "size"]],
  ["setCycle", "issue.set-cycle", "Set cycle", "issues", ["sprint"]],
  ["toggleSelect", "issue.select", "Select for bulk changes", "issues", ["bulk"]],
  ["peek", "issue.peek", "Peek without opening", "issues", ["preview"]],
  ["copyBranchName", "issue.copy-branch", "Copy the Git branch name", "issues", ["git"]],
];

const KEYS: Readonly<Record<Simple, string>> = {
  newIssue: "c",
  newIssueFromTemplate: "alt+c",
  quickAddTask: "q",
  setStatus: "s",
  setPriority: "p",
  setLabels: "l",
  assign: "a",
  assignToMe: "i",
  setEstimate: "e",
  setCycle: "shift+c",
  toggleSelect: "x",
  peek: "space",
  copyBranchName: "mod+shift+.",
};

const DESTINATIONS: readonly (readonly [Destination, string, string])[] = [
  ["inbox", "g i", "Go to Inbox"],
  ["my-work", "g m", "Go to My Work"],
  ["board", "g b", "Go to the board"],
  ["cycle", "g c", "Go to the current cycle"],
  ["triage", "g t", "Go to triage"],
];

/** The Work commands for the actions a screen supports. */
export function workCommands(actions: WorkActions): Command[] {
  const commands: Command[] = [];
  for (const [action, id, title, scope, keywords] of SPECS) {
    const run = actions[action];
    if (run !== undefined) {
      commands.push({ id, title, scope, keys: [KEYS[action]], keywords: keywords ?? [], run });
    }
  }
  const { goTo, destinations } = actions;
  if (goTo !== undefined) {
    for (const [destination, key, title] of DESTINATIONS) {
      if (destinations !== undefined && !destinations.includes(destination)) continue;
      commands.push({
        id: `go.${destination}`,
        title,
        scope: "global",
        keys: [key],
        keywords: ["open", "navigate"],
        run: () => {
          goTo(destination);
        },
      });
    }
  }
  return commands;
}
