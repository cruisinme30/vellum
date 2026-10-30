// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps } from "react";
import { useNavigate } from "react-router";

import type { Answer } from "../api/api.gen";
import { failureText } from "../data/failure";
import { useServerApi } from "../data/server-apis";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { AnswerView, appPath, SetUpModel, useModel } from "./answer";
import type { AssistApi } from "./api";
import { toSettings, type StoredModel } from "./settings";

import "./assist.css";

/**
 * The page bar's assistant button and the panel it opens: summarise the
 * page, or draft a new page that follows it as a template. Both are absent
 * in demo mode, which has no server to call a model from.
 */
export function usePageAssistant(pageId: string) {
  const t = useT();
  const api = useServerApi("assist");
  const [openFor, setOpenFor] = useState<string>();
  const open = openFor === pageId;
  if (api === undefined) {
    return { button: undefined, panel: undefined };
  }
  const button = (
    <button
      type="button"
      className="vl-icon-button"
      aria-label={t("assist-open")}
      title={t("assist-open")}
      aria-expanded={open}
      onClick={() => {
        setOpenFor(open ? undefined : pageId);
      }}
    >
      <Icon name="lightbulb" />
    </button>
  );
  const panel = open && (
    <PageAssistant
      key={pageId}
      api={api}
      pageId={pageId}
      onClose={() => {
        setOpenFor(undefined);
      }}
    />
  );
  return { button, panel };
}

/** The panel; exported for tests. */
export function PageAssistant({
  api,
  pageId,
  onClose,
}: {
  readonly api: AssistApi;
  readonly pageId: string;
  readonly onClose: () => void;
}) {
  const t = useT();
  const { stored } = useModel();
  return (
    <aside className="vl-assist-panel" aria-label={t("assist-open")}>
      <header className="vl-assist-panel-head">
        <h2>{t("assist-open")}</h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("assist-close")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </header>
      {stored === undefined ? (
        <SetUpModel />
      ) : (
        <>
          <Summary api={api} pageId={pageId} stored={stored} />
          <DraftFromTemplate api={api} pageId={pageId} stored={stored} />
        </>
      )}
    </aside>
  );
}

function Summary({
  api,
  pageId,
  stored,
}: {
  readonly api: AssistApi;
  readonly pageId: string;
  readonly stored: StoredModel;
}) {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState<Answer>();
  const [error, setError] = useState<string>();
  const run = () => {
    setBusy(true);
    setError(undefined);
    api
      .summarize(toSettings(stored), pageId)
      .then(setAnswer, (error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <section aria-labelledby="assist-summary-heading">
      <h3 id="assist-summary-heading" className="vl-assist-subhead">
        {t("assist-summary")}
      </h3>
      <button type="button" className="vl-filled-button" disabled={busy} onClick={run}>
        {busy ? t("assist-thinking") : t("assist-summarize")}
      </button>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      {answer !== undefined && <AnswerView answer={answer} />}
    </section>
  );
}

function DraftFromTemplate({
  api,
  pageId,
  stored,
}: {
  readonly api: AssistApi;
  readonly pageId: string;
  readonly stored: StoredModel;
}) {
  const t = useT();
  const navigate = useNavigate();
  const [title, setTitle] = useState("");
  const [instructions, setInstructions] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    api
      .draft(toSettings(stored), { templatePageId: pageId, title, instructions })
      .then(
        (draft) => {
          const path = appPath(draft.url);
          if (path.startsWith("/")) {
            void navigate(path);
          } else {
            void navigate(`/p/${encodeURIComponent(draft.page_id)}`);
          }
        },
        (error_: unknown) => {
          setError(failureText(error_));
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <section aria-labelledby="assist-draft-heading">
      <h3 id="assist-draft-heading" className="vl-assist-subhead">
        {t("assist-draft")}
      </h3>
      <p className="vl-muted">{t("assist-draft-intro")}</p>
      <form className="vl-assist-ask" onSubmit={submit}>
        <label>
          {t("assist-draft-title")}
          <input
            value={title}
            required
            maxLength={200}
            onChange={(event) => {
              setTitle(event.target.value);
            }}
          />
        </label>
        <label>
          {t("assist-draft-instructions")}
          <textarea
            value={instructions}
            required
            rows={4}
            maxLength={4000}
            onChange={(event) => {
              setInstructions(event.target.value);
            }}
          />
        </label>
        <button
          type="submit"
          className="vl-filled-button"
          disabled={busy || title.trim() === "" || instructions.trim() === ""}
        >
          {busy ? t("assist-thinking") : t("assist-draft-create")}
        </button>
      </form>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
