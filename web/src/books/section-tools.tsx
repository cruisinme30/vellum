// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";
import { useNavigate } from "react-router";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";

import { Failure, formText, useAction, type Submit } from "./actions";
import type { BookInfo, BooksApi, SectionDocument } from "./api";

type ToolMode = "add" | "child" | "rename" | "transfer" | "remove" | undefined;

/** What an editor can do to the section being read. */
export function SectionTools({
  api,
  info,
  section,
  hasChildren,
  onChanged,
  sectionPath,
}: {
  readonly api: BooksApi;
  readonly info: BookInfo;
  readonly section: SectionDocument;
  readonly hasChildren: boolean;
  /** Reloads the book; resolves when it has. */
  readonly onChanged: () => Promise<void>;
  readonly sectionPath: (book: string, section: string) => string;
}) {
  const t = useT();
  const navigate = useNavigate();
  const [mode, setMode] = useState<ToolMode>();
  const { busy, error, run } = useAction();
  const formId = useId();
  const book = info.id;
  const after = async (target: {
    book_id: string;
    section_id?: string;
    id?: string;
  }) => {
    await onChanged();
    setMode(undefined);
    const id = target.section_id ?? target.id;
    if (id !== undefined) void navigate(sectionPath(target.book_id, id));
  };
  const books = useLoaded(
    () => (mode === "transfer" ? api.list() : Promise.resolve([])),
    [api, mode],
    failureText,
  );

  const submit: Submit = (event) => {
    event.preventDefault();
    const title = formText(event.currentTarget, "title");
    switch (mode) {
      case "add":
      case "child": {
        if (title === "") return;
        const place =
          mode === "add"
            ? ({ at: "after", section: section.id } as const)
            : ({ at: "last_child_of", section: section.id } as const);
        run(async () => {
          await after(await api.addSection(book, title, place));
        });
        return;
      }
      case "rename": {
        if (title === "" || title === section.title) return;
        run(async () => {
          await api.renameSection(book, section.id, title);
          await onChanged();
          setMode(undefined);
        });
        return;
      }
      case "transfer": {
        const to = formText(event.currentTarget, "to");
        const copy = formText(event.currentTarget, "how") === "copy";
        if (to === "") return;
        run(async () => {
          await after(await api.transferSection(book, section.id, to, copy));
        });
        return;
      }
      case "remove": {
        run(async () => {
          await api.removeSection(book, section.id);
          await onChanged();
          setMode(undefined);
          void navigate(`/books/${encodeURIComponent(book)}`);
        });
        return;
      }
      case undefined: {
        return;
      }
    }
  };

  const toggle = (next: ToolMode) => {
    setMode((current) => (current === next ? undefined : next));
  };

  return (
    <div className="vl-book-tools">
      <div
        className="vl-book-toolbar"
        role="toolbar"
        aria-label={t("books-section-tools")}
      >
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={mode === "add"}
          onClick={() => {
            toggle("add");
          }}
        >
          {t("books-add-after")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={mode === "child"}
          onClick={() => {
            toggle("child");
          }}
        >
          {t("books-add-child")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={mode === "rename"}
          onClick={() => {
            toggle("rename");
          }}
        >
          {t("books-rename")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            run(async () => {
              await after(
                await api.copySection(book, section.id, {
                  at: "after",
                  section: section.id,
                }),
              );
            });
          }}
        >
          {t("books-copy")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={mode === "transfer"}
          onClick={() => {
            toggle("transfer");
          }}
        >
          {t("books-transfer")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            run(async () => {
              await api.splitSection(book, section.id);
              await onChanged();
            });
          }}
        >
          {t("books-split")}
        </button>
        {hasChildren && (
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            onClick={() => {
              run(async () => {
                await api.mergeSection(book, section.id);
                await onChanged();
              });
            }}
          >
            {t("books-merge")}
          </button>
        )}
        <button
          type="button"
          className="vl-text-button vl-danger"
          aria-expanded={mode === "remove"}
          onClick={() => {
            toggle("remove");
          }}
        >
          {t("books-remove")}
        </button>
      </div>
      {mode !== undefined && (
        <form className="vl-book-tool-form" id={formId} onSubmit={submit}>
          {(mode === "add" || mode === "child" || mode === "rename") && (
            <label>
              {mode === "rename"
                ? t("books-new-title")
                : t("books-section-title")}
              <input
                name="title"
                autoComplete="off"
                required
                defaultValue={mode === "rename" ? section.title : ""}
              />
            </label>
          )}
          {mode === "transfer" && (
            <Ready loaded={books}>
              {(list) => {
                const others = list.filter((b) => b.id !== book);
                if (others.length === 0)
                  return <p className="vl-muted">{t("books-no-other")}</p>;
                return (
                  <>
                    <label>
                      {t("books-transfer-to")}
                      <select name="to" required>
                        {others.map((b) => (
                          <option key={b.id} value={b.id}>
                            {b.title}
                          </option>
                        ))}
                      </select>
                    </label>
                    <fieldset className="vl-book-choice">
                      <legend>{t("books-transfer-how")}</legend>
                      <label>
                        <input
                          type="radio"
                          name="how"
                          value="move"
                          defaultChecked
                        />
                        {t("books-transfer-move")}
                      </label>
                      <label>
                        <input type="radio" name="how" value="copy" />
                        {t("books-transfer-copy")}
                      </label>
                    </fieldset>
                  </>
                );
              }}
            </Ready>
          )}
          {mode === "remove" && (
            <p>{t("books-remove-confirm", { title: section.title })}</p>
          )}
          <div className="vl-book-form-actions">
            <button type="submit" className="vl-filled-button" disabled={busy}>
              {mode === "remove" ? t("books-remove") : t("books-save")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                setMode(undefined);
              }}
            >
              {t("books-cancel")}
            </button>
          </div>
        </form>
      )}
      <Failure error={error} />
    </div>
  );
}

/** Adds the first section of an empty book. */
export function FirstSection({
  api,
  book,
  onChanged,
}: {
  readonly api: BooksApi;
  readonly book: string;
  readonly onChanged: () => Promise<void>;
}) {
  const t = useT();
  const navigate = useNavigate();
  const [error, setError] = useState<string>();
  return (
    <form
      className="vl-book-tool-form"
      onSubmit={(event) => {
        event.preventDefault();
        const title = new FormData(event.currentTarget).get("title");
        if (typeof title !== "string" || title.trim() === "") return;
        api.addSection(book, title.trim(), { at: "end" }).then(
          async (added) => {
            await onChanged();
            const bookPath = encodeURIComponent(book);
            void navigate(`/books/${bookPath}/${encodeURIComponent(added.id)}`);
          },
          (error_: unknown) => {
            setError(failureText(error_));
          },
        );
      }}
    >
      <label>
        {t("books-section-title")}
        <input name="title" autoComplete="off" required />
      </label>
      <button type="submit" className="vl-filled-button">
        {t("books-add-first")}
      </button>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
          <CopyDiagnostic message={error} />
        </p>
      )}
    </form>
  );
}
