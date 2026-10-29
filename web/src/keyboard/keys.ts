// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Key strokes and bindings. A binding is written the way the shortcut
 * sheet shows it: `mod+k` (⌘K on a Mac, Ctrl K elsewhere), `alt+c`,
 * `shift+c`, `?`, `space`, or a chord of two strokes such as `g i`.
 */

/** Which modifier `mod` means. */
export type KeyPlatform = "mac" | "other";

/** One key press, normalised. */
export interface Stroke {
  /** A lowercase letter or digit, a punctuation character, or a named key such as `space`. */
  readonly key: string;
  readonly mod: boolean;
  readonly alt: boolean;
  readonly shift: boolean;
}

/** One stroke, or a chord of two. */
export type Binding = readonly Stroke[];

/** The parts of a `KeyboardEvent` the keymap reads. */
export interface KeyLike {
  readonly key: string;
  readonly code?: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}

const NAMED: Readonly<Record<string, string>> = {
  " ": "space",
  Spacebar: "space",
  Enter: "enter",
  Escape: "escape",
  Esc: "escape",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Backspace: "backspace",
  Delete: "delete",
  Tab: "tab",
};

/** Punctuation read from the physical key when a shortcut modifier is held (⌘⇧. types > otherwise). */
const PUNCTUATION_CODES: Readonly<Record<string, string>> = {
  Period: ".",
  Comma: ",",
  Slash: "/",
  Semicolon: ";",
  BracketLeft: "[",
  BracketRight: "]",
};

const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "CapsLock", "Fn"]);

/** Letters and digits take shift into account; punctuation already carries it (`?` is shift+/). */
function isAlphanumeric(key: string): boolean {
  return /^[a-z0-9]$/.test(key);
}

function parseStroke(spec: string): Stroke {
  const parts = spec.toLowerCase().split("+");
  // `mod++` would split into an empty part; a literal plus is written `plus`.
  const key = parts.pop() ?? "";
  if (key === "" || parts.some((p) => !["mod", "alt", "shift"].includes(p))) {
    throw new Error(`Invalid key binding "${spec}"`);
  }
  const shift = parts.includes("shift");
  const modifier = parts.includes("mod");
  if (shift && !modifier && !isAlphanumeric(key) && key.length === 1) {
    throw new Error(`Write "${spec}" without shift; punctuation already carries it`);
  }
  return { key: key === "plus" ? "+" : key, mod: modifier, alt: parts.includes("alt"), shift };
}

/**
 * Parses a binding such as `mod+k` or `g i`.
 *
 * @throws Error for an unknown modifier, an empty key, or more than two strokes.
 */
export function parseBinding(spec: string): Binding {
  const strokes = spec
    .trim()
    .split(/\s+/)
    .map((s) => parseStroke(s));
  if (strokes.length > 2) {
    throw new Error(`Bindings have at most two strokes: "${spec}"`);
  }
  return strokes;
}

/**
 * The key an event names. Alt changes the character on a Mac (Alt C types
 * ç), so letters and digits come from the physical key, and so does
 * punctuation under the shortcut modifier (⌘⇧. would otherwise type >).
 */
function keyOf(event: KeyLike, modifier: boolean): { key: string; physicalPunctuation: boolean } {
  const code = event.code ?? "";
  const physical = /^(?:Key([A-Z])|Digit(\d))$/.exec(code);
  const letter = physical?.[1] ?? physical?.[2];
  if (letter !== undefined) {
    return { key: letter.toLowerCase(), physicalPunctuation: false };
  }
  const punctuation = modifier ? PUNCTUATION_CODES[code] : undefined;
  if (punctuation !== undefined) {
    return { key: punctuation, physicalPunctuation: true };
  }
  const key = NAMED[event.key] ?? event.key;
  return { key: key.length === 1 ? key.toLowerCase() : key, physicalPunctuation: false };
}

/**
 * The stroke a key event makes, or `undefined` for a lone modifier or a
 * modifier the platform does not use for shortcuts (Ctrl on a Mac, the
 * Windows key elsewhere), so system shortcuts pass through.
 */
export function strokeOf(event: KeyLike, platform: KeyPlatform): Stroke | undefined {
  const foreign = platform === "mac" ? event.ctrlKey : event.metaKey;
  if (MODIFIER_KEYS.has(event.key) || foreign) {
    return undefined;
  }
  const modifier = platform === "mac" ? event.metaKey : event.ctrlKey;
  const { key, physicalPunctuation } = keyOf(event, modifier);
  // Shift only counts for letters, digits, named keys and modified punctuation.
  const shift = event.shiftKey && (isAlphanumeric(key) || key.length > 1 || physicalPunctuation);
  return { key, mod: modifier, alt: event.altKey, shift };
}

/** Whether two strokes are the same. */
export function sameStroke(a: Stroke, b: Stroke): boolean {
  return a.key === b.key && a.mod === b.mod && a.alt === b.alt && a.shift === b.shift;
}

const MAC_GLYPHS: Readonly<Record<string, string>> = {
  space: "Space",
  enter: "↵",
  escape: "Esc",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
};

function formatStroke(stroke: Stroke, platform: KeyPlatform): string {
  const key = MAC_GLYPHS[stroke.key] ?? stroke.key.toUpperCase();
  if (platform === "mac") {
    return `${stroke.mod ? "⌘" : ""}${stroke.alt ? "⌥" : ""}${stroke.shift ? "⇧" : ""}${key}`;
  }
  const mods = [stroke.mod ? "Ctrl" : "", stroke.alt ? "Alt" : "", stroke.shift ? "Shift" : ""];
  return [...mods.filter(Boolean), key].join("+");
}

/** How a binding is shown: `⌘K`, `Ctrl+K`, `G then I`. */
export function formatBinding(binding: Binding, platform: KeyPlatform): string {
  return binding.map((s) => formatStroke(s, platform)).join(" then ");
}

/**
 * The platform of the running browser. The operating system's own name wins
 * over the user agent: WebKit browsers on Linux (GNOME Web, and Playwright's
 * WebKit) send a Safari-on-Mac user agent but use Ctrl for shortcuts.
 */
export function currentPlatform(
  nav: { readonly platform?: string; readonly userAgent?: string } = navigator,
): KeyPlatform {
  // An empty platform (some privacy modes) falls back to the user agent.
  const text =
    nav.platform === undefined || nav.platform === "" ? (nav.userAgent ?? "") : nav.platform;
  return /Mac|iPhone|iPad/.test(text) ? "mac" : "other";
}
