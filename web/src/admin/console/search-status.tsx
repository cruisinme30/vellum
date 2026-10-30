// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Admin → Settings → Search: what the server's search has (R-AI-7,
 * R-SR-5, D300). It names the index and the local embedding model, says
 * whether ranking by meaning is on and the model is in memory, and shows
 * how far a rebuild or an embedding backfill has got, checking again
 * every few seconds while one runs.
 */

import { useEffect, useState } from "react";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";

import type { SearchStatusView } from "./api";

/** How often the status is read again while a job runs. */
const POLL_MS = 3000;

const running = (status: SearchStatusView) =>
  status.job === "indexing" || status.job === "embedding";

function JobLine({ status }: { readonly status: SearchStatusView }) {
  const t = useT();
  const values = { done: status.done, total: status.total, error: status.error ?? "" };
  return (
    <>
      <p>{t(`admin-search-job-${status.job}`, values)}</p>
      {running(status) && (
        <progress
          aria-label={t("admin-search-progress")}
          max={Math.max(status.total, 1)}
          value={status.done}
        />
      )}
    </>
  );
}

/** The status panel; `refreshKey` reads it again (after a save). */
export function SearchStatusPanel({
  load,
  refreshKey,
}: {
  readonly load: () => Promise<SearchStatusView>;
  readonly refreshKey: unknown;
}) {
  const t = useT();
  const [status, setStatus] = useState<SearchStatusView>();
  const [failure, setFailure] = useState<string>();
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    load().then(
      (next) => {
        if (!current) return;
        setStatus(next);
        setFailure(undefined);
        if (running(next)) {
          timer = setTimeout(() => {
            setTick((n) => n + 1);
          }, POLL_MS);
        }
      },
      (error: unknown) => {
        if (current) setFailure(failureText(error));
      },
    );
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [load, refreshKey, tick]);

  return (
    <section className="vl-admin-search-status" aria-label={t("admin-search-status")}>
      {failure !== undefined && (
        <p className="vl-field-error" role="alert">
          {failure}
        </p>
      )}
      {status === undefined ? (
        failure === undefined && <p className="vl-muted">{t("admin-loading")}</p>
      ) : (
        <div role="status">
          <p>{t("admin-search-engine", { engine: status.engine })}</p>
          {status.model_configured ? (
            <>
              <p>
                {t("admin-search-model", {
                  model: status.model ?? "",
                  dimensions: status.dimensions ?? 0,
                })}
              </p>
              <p>{t(status.hybrid ? "admin-search-hybrid-on" : "admin-search-hybrid-off")}</p>
              <p>
                {status.loaded
                  ? t("admin-search-loaded")
                  : t("admin-search-unloaded", { minutes: status.idle_minutes ?? 15 })}
              </p>
            </>
          ) : (
            <p>{t("admin-search-no-model")}</p>
          )}
          <JobLine status={status} />
        </div>
      )}
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setTick((n) => n + 1);
        }}
      >
        {t("admin-search-refresh")}
      </button>
    </section>
  );
}
