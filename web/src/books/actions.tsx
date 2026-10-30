// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Small pieces the book panels share. */

import { useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";

/** A form's submit handler. */
export type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

const DATE = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/** A time as people read it. */
export const when = (at: string) => {
  const date = new Date(at);
  return Number.isNaN(date.getTime()) ? at : DATE.format(date);
};

/** Runs one action at a time, keeping its failure to show. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(undefined);
    action().then(
      () => {
        setBusy(false);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return { busy, error, run };
}

/** An action's failure, announced. */
export function Failure({ error }: { readonly error: string | undefined }) {
  if (error === undefined) return;
  return (
    <p className="vl-field-error" role="alert">
      {error}
    </p>
  );
}

/** A form field's trimmed text. */
export const formText = (form: HTMLFormElement, name: string) => {
  const value = new FormData(form).get(name);
  return typeof value === "string" ? value.trim() : "";
};
