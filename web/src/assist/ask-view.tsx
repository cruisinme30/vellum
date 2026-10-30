// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";
import { useSearchParams } from "react-router";

import type { Answer } from "../api/api.gen";
import { failureText } from "../data/failure";
import { ServerScreen } from "../data/server-apis";
import { useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { AnswerView, SetUpModel, useModel } from "./answer";
import type { AssistApi } from "./api";
import { toSettings } from "./settings";

import "./assist.css";

/** `/ask`: a question answered from what you can read, with citations. */
export function AskView() {
  const t = useT();
  return (
    <ServerScreen api="assist" title={t("assist-ask-title")}>
      {(api) => <Ask api={api} />}
    </ServerScreen>
  );
}

/** The form and the answer; exported for tests. */
export function Ask({ api }: { readonly api: AssistApi }) {
  const t = useT();
  const id = useId();
  const workspace = useWorkspace();
  const { stored } = useModel();
  const [parameters] = useSearchParams();
  const [question, setQuestion] = useState(parameters.get("q") ?? "");
  const [space, setSpace] = useState("");
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState<Answer>();
  const [error, setError] = useState<string>();

  if (stored === undefined) {
    return <SetUpModel />;
  }
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    if (question.trim() === "") return;
    setBusy(true);
    setError(undefined);
    setAnswer(undefined);
    api
      .ask(toSettings(stored), question.trim(), space === "" ? undefined : space)
      .then(setAnswer, (error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <>
      <p className="vl-muted">{t("assist-ask-intro", { model: stored.model })}</p>
      <form className="vl-assist-ask" onSubmit={submit}>
        <label htmlFor={`${id}-q`}>{t("assist-question")}</label>
        <textarea
          id={`${id}-q`}
          value={question}
          rows={3}
          maxLength={4000}
          onChange={(event) => {
            setQuestion(event.target.value);
          }}
        />
        <div className="vl-assist-row">
          <label>
            {t("assist-space")}{" "}
            <select
              value={space}
              onChange={(event) => {
                setSpace(event.target.value);
              }}
            >
              <option value="">{t("assist-space-all")}</option>
              {workspace.spaces.map((s) => (
                <option key={s.id} value={s.slug}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>
          <button
            type="submit"
            className="vl-filled-button"
            disabled={busy || question.trim() === ""}
          >
            {busy ? t("assist-thinking") : t("assist-ask")}
          </button>
        </div>
      </form>
      {busy && (
        <p className="vl-muted" role="status">
          {t("assist-thinking")}
        </p>
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      {answer !== undefined && <AnswerView answer={answer} />}
    </>
  );
}
