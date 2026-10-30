// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";

import { EXPORT_THEMES, SITE_THEMES } from "../admin/console/settings-page";
import { CHANNELS, KINDS } from "../notifications/preferences";
import { DESTINATIONS } from "../shell/destinations";
import { DENSITIES, FONTS, MOTIONS } from "../theme/appearance";
import { THEMES } from "../theme/themes";

import en from "./en.ftl?raw";
import { createTranslator, I18nProvider, useT } from "./i18n";

const SOURCE_ROOT = path.resolve(import.meta.dirname, "..");

/** Message ids defined in the English bundle. */
const DEFINED = new Set([...en.matchAll(/^([a-z][\w-]*)\s*=/gm)].map((match) => match[1]));

/** Every non-test source file, with its text. */
function sources(): [file: string, text: string][] {
  return readdirSync(SOURCE_ROOT, { recursive: true, encoding: "utf8" })
    .filter((file) => /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file))
    .map((file) => [file, readFileSync(path.join(SOURCE_ROOT, file), "utf8")]);
}

const literals = (text: string, pattern: RegExp) =>
  [...text.matchAll(pattern)].flatMap((match) => match[1] ?? []);

/** Ids passed to `t(...)` as string literals, including both arms of a ternary. */
function literalIds(text: string): string[] {
  return literals(text, /\bt\(([^,()]*)/g).flatMap((argument) => literals(argument, /"([^"]+)"/g));
}

/** Ids built from a prefix and a value: `t(\`nav-${id}\`)` and friends. */
function familyIds(): string[] {
  const toolbar = readFileSync(path.join(SOURCE_ROOT, "editor/toolbar.tsx"), "utf8");
  return [
    ...Object.values(DESTINATIONS).map((destination) => `nav-${destination.id}`),
    ...THEMES.map((theme) => `theme-${theme}`),
    "theme-custom",
    ...DENSITIES.map((density) => `density-${density}`),
    ...MOTIONS.map((motion) => `motion-${motion}`),
    ...FONTS.map((font) => `font-${font}`),
    ...["too-large", "not-json", "format", "name", "base", "seed"].map(
      (kind) => `theme-import-${kind}`,
    ),
    ...[...EXPORT_THEMES, ...SITE_THEMES].map((theme) => `admin-theme-${theme}`),
    ...KINDS.map((kind) => `notify-kind-${kind}`),
    ...CHANNELS.map((channel) => `notify-channel-${channel}`),
    ...["checking", "on", "off", "denied", "unsupported", "demo"].map(
      (state) => `notify-device-${state}`,
    ),
    ...["draft", "in-review", "verified", "outdated"].map((status) => `status-${status}`),
    ...literals(toolbar, /\bid: "([\w-]+)"/g).map((id) => `toolbar-${id}`),
    // Parse errors of collection editors, returned by parseInput.
    ...literals(
      readFileSync(path.join(SOURCE_ROOT, "collections/value.ts"), "utf8"),
      /"(collection-error-[\w-]+)"/g,
    ),
  ];
}

describe("createTranslator", () => {
  const translate = createTranslator(
    "en",
    "hello = Hello, { $name }!\ncount = { $n ->\n [one] one page\n *[other] { $n } pages\n}\nonly-attr =\n .title = T\n",
  );

  it("formats messages with arguments and plurals", () => {
    expect(translate("hello", { name: "Priya" })).toBe("Hello, Priya!");
    expect(translate("count", { n: 1 })).toBe("one page");
    expect(translate("count", { n: 3 })).toBe("3 pages");
  });

  it("returns the id for unknown messages and messages without a value", () => {
    expect(translate("missing")).toBe("missing");
    expect(translate("only-attr")).toBe("only-attr");
  });

  it("throws when the source defines a message twice", () => {
    expect(() => createTranslator("xx", "a = 1\na = 2\n")).toThrow(/xx messages do not parse/);
  });
});

describe("useT", () => {
  it("translates English inside and outside the provider", () => {
    const wrapper = ({ children }: { readonly children: ReactNode }) => (
      <I18nProvider>{children}</I18nProvider>
    );
    expect(renderHook(() => useT(), { wrapper }).result.current("nav-home")).toBe("Home");
    expect(renderHook(() => useT()).result.current("nav-home")).toBe("Home");
  });
});

describe("the English bundle", () => {
  it("parses", () => {
    expect(() => createTranslator("en", en)).not.toThrow();
    expect(DEFINED.size).toBeGreaterThan(50);
  });

  it("defines every id the source passes to t()", () => {
    const missing = sources().flatMap(([file, text]) =>
      literalIds(text)
        .filter((id) => !DEFINED.has(id))
        .map((id) => `${file}: ${id}`),
    );
    expect(missing).toEqual([]);
  });

  it("finds the literal ids it checks", () => {
    const found = sources().flatMap(([, text]) => literalIds(text));
    expect(found).toEqual(expect.arrayContaining(["tree-expand", "tree-collapse", "toc-empty"]));
  });

  it("defines every id built from a prefix or listed in a table", () => {
    const tables = ["shell/page-menu.tsx", "views/info.tsx"].flatMap((file) =>
      literals(readFileSync(path.join(SOURCE_ROOT, file), "utf8"), /"((?:menu|help-key)-[\w-]+)"/g),
    );
    const missing = [...familyIds(), ...tables].filter((id) => !DEFINED.has(id));
    expect(missing).toEqual([]);
  });
});
