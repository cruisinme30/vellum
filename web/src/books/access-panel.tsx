// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useServerApi } from "../data/server-apis";
import { useT } from "../i18n/i18n";
import { PeoplePicker } from "../people/people-picker";

import { Failure, useAction, type Submit } from "./actions";
import type { BooksApi, SectionDocument } from "./api";

type Access = NonNullable<SectionDocument["access"]>;

function AccessForm({
  api,
  book,
  section,
  access,
  onChanged,
}: {
  readonly api: BooksApi;
  readonly book: string;
  readonly section: SectionDocument;
  readonly access: Access;
  readonly onChanged: () => Promise<void>;
}) {
  const t = useT();
  const directory = useServerApi("directory");
  const { busy, error, run } = useAction();
  const [saved, setSaved] = useState(false);
  const [restrictTo, setRestrictTo] = useState<string[]>(() => [
    ...(access.own_restrict_to ?? []),
  ]);
  const [owners, setOwners] = useState<string[]>(() => [...access.owners]);
  const [reviewers, setReviewers] = useState<string[]>(() => [
    ...access.reviewers,
  ]);
  const changed =
    <T,>(set: (value: T) => void) =>
    (value: T) => {
      set(value);
      setSaved(false);
    };
  const submit: Submit = (event) => {
    event.preventDefault();
    setSaved(false);
    run(async () => {
      await api.setAccess(book, section.id, { restrictTo, owners, reviewers });
      await onChanged();
      setSaved(true);
    });
  };
  return (
    <form className="vl-book-tool-form" onSubmit={submit}>
      <PeoplePicker
        label={t("books-access-restrict")}
        value={restrictTo}
        onChange={changed(setRestrictTo)}
        directory={directory}
        hint={t("books-access-help")}
      />
      <PeoplePicker
        label={t("books-access-owners")}
        value={owners}
        onChange={changed(setOwners)}
        directory={directory}
      />
      <PeoplePicker
        label={t("books-access-reviewers")}
        value={reviewers}
        onChange={changed(setReviewers)}
        directory={directory}
      />
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
      <Failure error={error} />
    </form>
  );
}

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
  const access = section.access;
  if (access == undefined) return;
  const inherited = access.restricted_by.filter((id) => id !== section.id);
  return (
    <section className="vl-book-panel" aria-labelledby="vl-book-access-title">
      <h2 id="vl-book-access-title">
        {t("books-access-title", { title: section.title })}
      </h2>
      {inherited.length > 0 && (
        <p className="vl-muted">{t("books-access-inherited")}</p>
      )}
      {access.restrict_to != undefined && (
        <p className="vl-muted">
          {t("books-access-effective", { who: access.restrict_to.join(", ") })}
        </p>
      )}
      <AccessForm
        key={section.id}
        api={api}
        book={book}
        section={section}
        access={access}
        onChanged={onChanged}
      />
    </section>
  );
}
