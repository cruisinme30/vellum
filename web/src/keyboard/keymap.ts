// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The keyboard model: commands with bindings, grouped by where they apply,
 * and a dispatcher that turns key events into commands. Dispatch is a
 * synchronous lookup so a key press reaches its command in well under the
 * 100 ms budget for perceived latency.
 */

import {
  parseBinding,
  sameStroke,
  strokeOf,
  type Binding,
  type KeyLike,
  type KeyPlatform,
  type Stroke,
} from "./keys";

/** Where a command applies. More specific scopes win over `global`. */
export type Scope = "global" | "issues" | "editor";

/** Scopes from most to least specific. */
const SCOPES: readonly Scope[] = ["editor", "issues", "global"];

/** Something a person can do from the keyboard or the command menu. */
export interface Command {
  /** A stable ID, such as `issue.create`. */
  readonly id: string;
  /** What the command menu and the shortcut sheet show. */
  readonly title: string;
  /** Bindings, as `parseBinding` reads them. A command may have none and still be in the menu. */
  readonly keys: readonly string[];
  readonly scope: Scope;
  /** Extra words the command menu matches, such as "ticket" for new issue. */
  readonly keywords?: readonly string[];
  /** Runs the command. */
  readonly run: () => void;
}

/** What a key press did. */
export type Dispatch =
  | { readonly kind: "run"; readonly command: Command }
  | { readonly kind: "pending" }
  | { readonly kind: "none" };

interface Bound {
  readonly command: Command;
  readonly binding: Binding;
}

/** Options for a keymap. */
export interface KeymapOptions {
  readonly platform: KeyPlatform;
  /** How long the second stroke of a chord may wait, in milliseconds. */
  readonly chordTimeoutMs?: number;
  /** The clock, for tests. */
  readonly now?: () => number;
}

/** Whether the event target takes typing, where plain keys must not run commands. */
export function isEditable(target: unknown): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement)) {
    return false;
  }
  if (
    target.isContentEditable ||
    target.closest("[contenteditable=''], [contenteditable='true']")
  ) {
    return true;
  }
  return ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

function key(binding: Binding): string {
  return binding
    .map((s) => `${s.mod ? "m" : ""}${s.alt ? "a" : ""}${s.shift ? "s" : ""}:${s.key}`)
    .join(" ");
}

/** Commands and their bindings, checked for clashes. */
export class Keymap {
  readonly #bound: readonly Bound[];
  readonly #timeout: number;
  readonly #now: () => number;
  #pending: { readonly stroke: Stroke; readonly at: number } | undefined;

  /**
   * @throws Error when two commands share a binding in one scope, or a
   *   single-stroke binding is also the first stroke of a chord in a scope
   *   that can be active with it.
   */
  constructor(
    readonly commands: readonly Command[],
    readonly options: KeymapOptions,
  ) {
    this.#timeout = options.chordTimeoutMs ?? 1500;
    this.#now = options.now ?? (() => performance.now());
    this.#bound = commands.flatMap((command) =>
      command.keys.map((k) => ({ command, binding: parseBinding(k) })),
    );
    const seen = new Map<string, string>();
    for (const { command, binding } of this.#bound) {
      const id = `${command.scope} ${key(binding)}`;
      const other = seen.get(id);
      if (other !== undefined) {
        throw new Error(
          `${command.id} and ${other} share the binding ${key(binding)} in ${command.scope}`,
        );
      }
      seen.set(id, command.id);
    }
    for (const chord of this.#bound.filter((b) => b.binding.length === 2)) {
      const clash = this.#bound.find(
        (b) =>
          b.binding.length === 1 &&
          b.binding[0] !== undefined &&
          chord.binding[0] !== undefined &&
          sameStroke(b.binding[0], chord.binding[0]) &&
          (b.command.scope === chord.command.scope ||
            b.command.scope === "global" ||
            chord.command.scope === "global"),
      );
      if (clash) {
        throw new Error(`${clash.command.id} takes the first key of ${chord.command.id}`);
      }
    }
  }

  /** The bindings of a command, for display. */
  bindingsOf(command: Command): Binding[] {
    return this.#bound.filter((b) => b.command === command).map((b) => b.binding);
  }

  /** Whether a chord is waiting for its second stroke. */
  get pending(): boolean {
    return this.#pending !== undefined && this.#now() - this.#pending.at <= this.#timeout;
  }

  /** Forgets a half-typed chord. */
  reset(): void {
    this.#pending = undefined;
  }

  #find(strokes: readonly Stroke[], active: ReadonlySet<Scope>): Command | undefined {
    for (const scope of SCOPES) {
      if (!active.has(scope)) {
        continue;
      }
      const hit = this.#bound.find(
        (b) =>
          b.command.scope === scope &&
          b.binding.length === strokes.length &&
          b.binding.every((s, index) => {
            const other = strokes[index];
            return other !== undefined && sameStroke(s, other);
          }),
      );
      if (hit) {
        return hit.command;
      }
    }
    return undefined;
  }

  /**
   * Turns a key event into a command. `active` holds the scopes in force
   * where the event happened; `global` should always be in it.
   */
  dispatch(event: KeyLike & { readonly target?: unknown }, active: ReadonlySet<Scope>): Dispatch {
    const stroke = strokeOf(event, this.options.platform);
    if (stroke === undefined) {
      return { kind: "none" };
    }
    if (!stroke.mod && isEditable(event.target)) {
      this.reset();
      return { kind: "none" };
    }
    const first = this.pending ? this.#pending?.stroke : undefined;
    this.reset();
    if (first !== undefined) {
      const chord = this.#find([first, stroke], active);
      return chord ? { kind: "run", command: chord } : { kind: "none" };
    }
    const single = this.#find([stroke], active);
    if (single) {
      return { kind: "run", command: single };
    }
    const starts = this.#bound.some(
      (b) =>
        b.binding.length === 2 &&
        active.has(b.command.scope) &&
        b.binding[0] !== undefined &&
        sameStroke(b.binding[0], stroke),
    );
    if (starts) {
      this.#pending = { stroke, at: this.#now() };
      return { kind: "pending" };
    }
    return { kind: "none" };
  }
}
