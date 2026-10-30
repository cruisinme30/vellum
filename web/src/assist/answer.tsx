// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import type { Answer } from "../api/api.gen";
import { useSession } from "../api/session";
import { MarkdownReader } from "../books/markdown";
import { useT } from "../i18n/i18n";

import { useStoredModel, type StoredModel } from "./settings";

import "./assist.css";

/** A link inside the app for an absolute app URL, else the URL. */
export function appPath(url: string): string {
  try {
    const parsed = new URL(url, globalThis.location.origin);
    return parsed.origin === globalThis.location.origin
      ? `${parsed.pathname}${parsed.search}${parsed.hash}`
      : url;
  } catch {
    return url;
  }
}

/** A model's answer: its Markdown, the sources it cites, and caveats. */
export function AnswerView({ answer }: { readonly answer: Answer }) {
  const t = useT();
  return (
    <div className="vl-assist-answer" aria-live="polite">
      <MarkdownReader markdown={answer.text} />
      {answer.citations.length > 0 && (
        <section aria-label={t("assist-sources")}>
          <h3 className="vl-assist-subhead">{t("assist-sources")}</h3>
          <ol className="vl-assist-sources">
            {answer.citations.map((c) => (
              <li key={c.n} value={c.n}>
                <Link to={appPath(c.url)}>{c.title}</Link>
                <p className="vl-muted">{c.snippet}</p>
              </li>
            ))}
          </ol>
        </section>
      )}
      <p className="vl-muted vl-assist-meta">
        {answer.truncated ? `${t("assist-truncated")} ` : ""}
        {t("assist-written-by", { model: answer.model })}
      </p>
    </div>
  );
}

/** The signed-in person's id and saved model, or a prompt to set one up. */
export function useModel(): { userId: string | undefined; stored: StoredModel | undefined } {
  const session = useSession();
  const userId = session?.status.user?.id;
  return { userId, stored: useStoredModel(userId) };
}

/** Shown where a feature needs a model and none is set up. */
export function SetUpModel() {
  const t = useT();
  return (
    <p className="vl-muted">
      {t("assist-no-model")} <Link to="/settings/ai">{t("assist-set-up")}</Link>
    </p>
  );
}
