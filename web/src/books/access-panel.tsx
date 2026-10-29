// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";

import { Failure, formText, useAction, type Submit } from "./actions";
import type { BooksApi, SectionDocument } from "./api";
import { principalList } from "./outline";

/** Who may see, own and review the section (book managers only). */
export function AccessPanel({
  api,
  book,
  section,
  onChanged,
}: {
  readonly api: BooksApi;
  readonly book: string;
  readonly section: SectionDocument;
  readonly onChanged: () => Promise<void>;
}) {
  const t = useT();
  const { busy, error, run } = useAction();
  const [saved, setSaved] = useState(false);
  const access = section.access;
  if (access == undefined) return;
  const inherited = access.restricted_by.filter((id) => id !== section.id);
  const submit: Submit = (event) => {
    event.preventDefault();
    const value = {
      restrictTo: principalList(formText(event.currentTarget, "restrict")),
      owners: principalList(formText(event.currentTarget, "owners")),
      reviewers: principalList(formText(event.currentTarget, "reviewers")),
    };
    setSaved(false);
    run(async () => {
      await api.setAccess(book, section.id, value);
      await onChanged();
      setSaved(true);
    });
  };
  return (
    <section className="vl-book-panel" aria-labelledby="vl-book-access-title">
      <h2 id="vl-book-access-title">{t("books-access-title", { title: section.title })}</h2>
      {inherited.length > 0 && <p className="vl-muted">{t("books-access-inherited")}</p>}
      {access.restrict_to != undefined && (
        <p className="vl-muted">
          {t("books-access-effective", { who: access.restrict_to.join(", ") })}
        </p>
      )}
      <form className="vl-book-tool-form" onSubmit={submit} key={section.id}>
        <label>
          {t("books-access-restrict")}
          <textarea
            name="restrict"
            rows={2}
            defaultValue={(access.own_restrict_to ?? []).join("\n")}
            aria-describedby="vl-book-access-help"
          />
        </label>
        <label>
          {t("books-access-owners")}
          <textarea name="owners" rows={2} defaultValue={access.owners.join("\n")} />
        </label>
        <label>
          {t("books-access-reviewers")}
          <textarea name="reviewers" rows={2} defaultValue={access.reviewers.join("\n")} />
        </label>
        <p id="vl-book-access-help" className="vl-muted">
          {t("books-access-help")}
        </p>
        <div className="vl-book-form-actions">
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("books-save")}
          </button>
          {saved && (
            <span className="vl-muted" role="status">
              {t("books-saved")}
            </span>
          )}
        </div>
      </form>
      <Failure error={error} />
    </section>
  );
}
