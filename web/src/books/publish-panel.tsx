// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../data/failure";
import { Ready, useServerApi } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { PeoplePicker } from "../people/people-picker";

import { Failure, formText, useAction, when, type Submit } from "./actions";
import type { BooksApi, Publication, PublishFormat } from "./api";

function PublicationLine({
  publication,
}: {
  readonly publication: Publication;
}) {
  const t = useT();
  return (
    <li>
      <strong>
        {publication.format === "site"
          ? t("books-publish-site")
          : t("books-publish-mdbook")}
      </strong>{" "}
      <small className="vl-muted">
        {publication.public
          ? t("books-publish-public")
          : t("books-publish-members")}{" "}
        · {when(publication.at)} ·{" "}
        {t("books-publish-files", { count: publication.files })}
      </small>
      <div className="vl-muted">{publication.folder}</div>
      {publication.url != undefined && (
        <a href={publication.url} target="_blank" rel="noopener noreferrer">
          {t("books-publish-open")}
        </a>
      )}
    </li>
  );
}

/** Publishing the book as a static site or an mdBook tree (managers only). */
export function PublishPanel({
  api,
  book,
}: {
  readonly api: BooksApi;
  readonly book: string;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [members, setMembers] = useState(false);
  const [audience, setAudience] = useState<string[]>([]);
  const [missing, setMissing] = useState(false);
  const directory = useServerApi("directory");
  const loaded = useLoaded(
    () => api.publications(book),
    [api, book, version],
    failureText,
  );
  const { busy, error, run } = useAction();
  const submit: Submit = (event) => {
    event.preventDefault();
    const format = formText(event.currentTarget, "format") as PublishFormat;
    // An empty audience would publish for everyone, so say so instead.
    setMissing(members && audience.length === 0);
    if (members && audience.length === 0) return;
    run(async () => {
      await api.publish(book, format, members ? audience : []);
      setVersion((v) => v + 1);
    });
  };
  return (
    <section className="vl-book-panel" aria-labelledby="vl-book-publish-title">
      <h2 id="vl-book-publish-title">{t("books-publish")}</h2>
      <form className="vl-book-tool-form" onSubmit={submit}>
        <fieldset className="vl-book-choice">
          <legend>{t("books-publish-format")}</legend>
          <label>
            <input type="radio" name="format" value="site" defaultChecked />
            {t("books-publish-site")}
          </label>
          <label>
            <input type="radio" name="format" value="mdbook" />
            {t("books-publish-mdbook")}
          </label>
        </fieldset>
        <fieldset className="vl-book-choice">
          <legend>{t("books-publish-audience")}</legend>
          <label>
            <input
              type="radio"
              name="who"
              checked={!members}
              onChange={() => {
                setMembers(false);
              }}
            />
            {t("books-publish-everyone")}
          </label>
          <label>
            <input
              type="radio"
              name="who"
              checked={members}
              onChange={() => {
                setMembers(true);
              }}
            />
            {t("books-publish-some")}
          </label>
        </fieldset>
        {members && (
          <PeoplePicker
            label={t("books-publish-who")}
            value={audience}
            onChange={setAudience}
            directory={directory}
          />
        )}
        <p id="vl-book-publish-help" className="vl-muted">
          {t("books-publish-help")}
        </p>
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("books-publish-now")}
        </button>
      </form>
      <Failure error={missing ? t("books-publish-who-needed") : error} />
      <Ready loaded={loaded}>
        {(publications) =>
          publications.length === 0 ? (
            <p className="vl-muted">{t("books-publish-none")}</p>
          ) : (
            <ul className="vl-book-entries">
              {publications.map((p) => (
                <PublicationLine key={p.format} publication={p} />
              ))}
            </ul>
          )
        }
      </Ready>
    </section>
  );
}
