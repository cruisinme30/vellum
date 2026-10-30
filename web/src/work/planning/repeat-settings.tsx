// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * An issue's due date and repeat (#144, R-WK-12): set, change or stop how
 * the issue repeats. Saved in one change, so the due date a repeat needs
 * lands with it. Shows only the repeat line without a server.
 */

import { useId, useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useServerApi } from "../../data/server-apis";
import { useT } from "../../i18n/i18n";
import { repeatChoices } from "../templates/template-file";

import type { IssueOut, PlanningApi } from "./api";

function Form({
  api,
  issue,
  onSaved,
}: {
  readonly api: PlanningApi;
  readonly issue: IssueOut;
  readonly onSaved: () => void;
}) {
  const t = useT();
  const id = useId();
  const [due, setDue] = useState(issue.due ?? "");
  const [repeat, setRepeat] = useState(issue.repeat ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const dirty = due !== (issue.due ?? "") || repeat !== (issue.repeat ?? "");
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    setSaved(false);
    if (repeat !== "" && due === "") {
      setError(t("plan-repeat-due-needed"));
      return;
    }
    setBusy(true);
    setError(undefined);
    api.setRepeat(issue.key, repeat, due).then(
      () => {
        setBusy(false);
        setSaved(true);
        onSaved();
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return (
    <form
      className="vl-work-filters vl-plan-repeat"
      aria-label={t("plan-repeat")}
      onSubmit={submit}
    >
      <label htmlFor={`${id}-due`}>{t("plan-due-date")}</label>
      <input
        id={`${id}-due`}
        type="date"
        value={due}
        onChange={(event) => {
          setDue(event.target.value);
        }}
      />
      <label htmlFor={`${id}-repeat`}>{t("plan-repeat")}</label>
      <select
        id={`${id}-repeat`}
        value={repeat}
        aria-describedby={`${id}-hint`}
        onChange={(event) => {
          setRepeat(event.target.value);
        }}
      >
        <option value="">{t("plan-repeat-never")}</option>
        {repeatChoices(repeat).map((r) => (
          <option key={r} value={r}>
            {t("plan-repeat-every", { repeat: r })}
          </option>
        ))}
      </select>
      <button type="submit" className="vl-text-button" disabled={busy || !dirty}>
        {t("plan-repeat-save")}
      </button>
      <p role="status" className="vl-muted">
        {saved && !dirty ? t("plan-repeat-saved") : ""}
      </p>
      <small id={`${id}-hint`} className="vl-muted">
        {t("plan-repeat-hint")}
      </small>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** The repeat line, editable when a server is there. */
export function RepeatSettings({
  issue,
  onSaved,
}: {
  readonly issue: IssueOut;
  readonly onSaved: () => void;
}) {
  const t = useT();
  const api = useServerApi("planning");
  if (api === undefined) {
    if (issue.repeat == undefined) return;
    return <p className="vl-muted">{t("plan-repeats", { repeat: issue.repeat })}</p>;
  }
  return <Form api={api} issue={issue} onSaved={onSaved} />;
}
