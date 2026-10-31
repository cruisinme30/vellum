// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Task lists (R-MW-7): personal lists and lists shared with other people,
 * which Google Tasks lacks. The owner shares a list by handle; members see
 * it in their own My Work and can be assigned its tasks.
 */

import { useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { ListView, WorkApi } from "./api";

/** Reads one trimmed text field of a submitted form, then clears the form. */
function take(
  event: Parameters<NonNullable<ComponentProps<"form">["onSubmit"]>>[0],
  name: string,
) {
  event.preventDefault();
  const form = event.currentTarget;
  const value = new FormData(form).get(name);
  const text = typeof value === "string" ? value.trim() : "";
  if (text !== "") form.reset();
  return text;
}

/** One list: its members, and for its owner, sharing controls. */
function ListRow({
  api,
  list,
  mine,
  run,
}: {
  readonly api: WorkApi;
  readonly list: ListView;
  readonly mine: boolean;
  readonly run: (write: () => Promise<unknown>) => void;
}) {
  const t = useT();
  return (
    <li className="vl-list-row">
      <p>
        <strong>{list.title}</strong>{" "}
        <small className="vl-muted">
          {t("my-work-list-open", { count: list.open })}
        </small>
        {!mine && list.owner != undefined && (
          <small className="vl-muted">
            {" "}
            · {t("my-work-list-shared-by", { owner: list.owner })}
          </small>
        )}
      </p>
      {list.members.length > 0 && (
        <ul
          className="vl-list-members"
          aria-label={t("my-work-list-members", { list: list.title })}
        >
          {list.members.map((member) => (
            <li key={member}>
              @{member}
              {mine && (
                <button
                  type="button"
                  className="vl-text-button"
                  aria-label={t("my-work-list-unshare", {
                    member,
                    list: list.title,
                  })}
                  onClick={() => {
                    run(() => api.shareList(list.doc, member, false));
                  }}
                >
                  ×
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {mine && (
        <form
          className="vl-list-share"
          onSubmit={(event) => {
            const member = take(event, "member");
            if (member !== "") run(() => api.shareList(list.doc, member, true));
          }}
        >
          <label>
            {t("my-work-list-share", { list: list.title })}
            <input
              name="member"
              autoComplete="off"
              placeholder="@handle"
              required
            />
          </label>
          <button type="submit" className="vl-text-button">
            {t("my-work-list-share-button")}
          </button>
        </form>
      )}
    </li>
  );
}

/** The Lists panel under My Work. */
export function ListsPanel({
  api,
  onChanged,
}: {
  readonly api: WorkApi;
  readonly onChanged: () => void;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(() => api.lists(), [api, version], failureText);
  const run = (write: () => Promise<unknown>) => {
    setError(undefined);
    write().then(
      () => {
        setVersion((v) => v + 1);
        onChanged();
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  return (
    <details className="vl-work-panel">
      <summary>{t("my-work-lists")}</summary>
      <Ready loaded={loaded}>
        {(lists) => {
          const me = lists.find((list) => list.inbox)?.owner ?? undefined;
          const shown = lists.filter((list) => !list.inbox);
          return shown.length === 0 ? (
            <p className="vl-muted">{t("my-work-lists-empty")}</p>
          ) : (
            <ul className="vl-list-list">
              {shown.map((list) => (
                <ListRow
                  key={list.doc}
                  api={api}
                  list={list}
                  mine={me !== undefined && list.owner === me}
                  run={run}
                />
              ))}
            </ul>
          );
        }}
      </Ready>
      <form
        className="vl-list-share"
        onSubmit={(event) => {
          const title = take(event, "title");
          if (title !== "") run(() => api.createList(title));
        }}
      >
        <label>
          {t("my-work-list-new")}
          <input name="title" autoComplete="off" required />
        </label>
        <button type="submit" className="vl-filled-button">
          {t("my-work-list-create")}
        </button>
      </form>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </details>
  );
}
