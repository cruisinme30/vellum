// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  FluentBundle,
  FluentResource,
  type FluentVariable,
} from "@fluent/bundle";
import { createContext, use, useMemo, type ReactNode } from "react";

import planning from "../work/planning/planning.ftl?raw";

import en from "./en.ftl?raw";

/** Formats a message by its Fluent id. Unknown ids come back unchanged. */
export type Translate = (
  id: string,
  arguments_?: Record<string, FluentVariable>,
) => string;

/** Builds a translator for one locale from Fluent source. */
export function createTranslator(locale: string, source: string): Translate {
  const bundle = new FluentBundle(locale, { useIsolating: false });
  const errors = bundle.addResource(new FluentResource(source));
  if (errors.length > 0) {
    throw new Error(
      `Vellum: ${locale} messages do not parse: ${errors.map(String).join("; ")}`,
    );
  }
  return (id, arguments_) => {
    const message = bundle.getMessage(id);
    if (message?.value == undefined) {
      return id;
    }
    return bundle.formatPattern(message.value, arguments_);
  };
}

// Each area may keep its messages next to its code; they join one bundle.
const english = createTranslator("en", `${en}\n${planning}`);
const I18nContext = createContext<Translate>(english);

/**
 * Provides the translator for the user's locale. English is the only locale so
 * far.
 */
export function I18nProvider({ children }: { readonly children: ReactNode }) {
  const translate = useMemo(() => english, []);
  return <I18nContext value={translate}>{children}</I18nContext>;
}

/** The translator for the current locale. */
export function useT(): Translate {
  return use(I18nContext);
}
