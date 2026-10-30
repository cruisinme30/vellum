// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../../data/failure";
import { formatSize } from "../../data/format";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";

import type { AdminConsoleApi, UsageCounts } from "./api";
import { AdminScreen } from "./context";

const TIME = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

const NUMBER = new Intl.NumberFormat("en");

type Figure = readonly [
  labelId: string,
  value: (counts: UsageCounts) => string,
];

const count = (pick: (c: UsageCounts) => number) => (c: UsageCounts) =>
  NUMBER.format(pick(c));

/** Each group of figures, in reading order. */
const GROUPS: readonly (readonly [
  headingId: string,
  figures: readonly Figure[],
])[] = [
  [
    "admin-usage-people",
    [
      ["admin-usage-users", count((c) => c.users)],
      ["admin-usage-enabled", count((c) => c.enabled_users)],
      ["admin-usage-admins", count((c) => c.admins)],
      ["admin-usage-sessions", count((c) => c.sessions)],
      ["admin-usage-tokens", count((c) => c.api_tokens)],
    ],
  ],
  [
    "admin-usage-activity",
    [
      ["admin-usage-active-1d", count((c) => c.active_users_1d)],
      ["admin-usage-active-7d", count((c) => c.active_users_7d)],
      ["admin-usage-active-30d", count((c) => c.active_users_30d)],
      ["admin-usage-events-1d", count((c) => c.events_1d)],
      ["admin-usage-events-7d", count((c) => c.events_7d)],
    ],
  ],
  [
    "admin-usage-content",
    [
      ["admin-usage-spaces", count((c) => c.spaces)],
      ["admin-usage-pages", count((c) => c.pages)],
      ["admin-usage-books", count((c) => c.books)],
      ["admin-usage-collections", count((c) => c.collections)],
      ["admin-usage-folders", count((c) => c.folders)],
      ["admin-usage-trashed", count((c) => c.trashed)],
      ["admin-usage-versions", count((c) => c.versions)],
    ],
  ],
  [
    "admin-usage-storage",
    [
      ["admin-usage-total", (c) => formatSize(c.file_bytes + c.version_bytes)],
      ["admin-usage-files", (c) => formatSize(c.file_bytes)],
      ["admin-usage-version-bytes", (c) => formatSize(c.version_bytes)],
    ],
  ],
];

function Figures({ counts }: { readonly counts: UsageCounts }) {
  const t = useT();
  return (
    <div className="vl-usage">
      {GROUPS.map(([headingId, figures]) => (
        <section
          key={headingId}
          aria-labelledby={headingId}
          className="vl-usage-group"
        >
          <h2 id={headingId}>{t(headingId)}</h2>
          <dl className="vl-usage-grid">
            {figures.map(([labelId, value]) => (
              <div key={labelId} className="vl-usage-tile">
                <dt>{t(labelId)}</dt>
                <dd>{value(counts)}</dd>
              </div>
            ))}
          </dl>
        </section>
      ))}
    </div>
  );
}

function Overview({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const [refresh, setRefresh] = useState(0);
  const loaded = useLoaded(
    () => api.usage(refresh > 0),
    [api, refresh],
    failureText,
  );
  const counts = loaded.state === "ready" ? loaded.value : undefined;
  return (
    <>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          disabled={loaded.state === "loading"}
          onClick={() => {
            setRefresh((n) => n + 1);
          }}
        >
          {loaded.state === "loading"
            ? t("admin-usage-counting")
            : t("admin-usage-count-now")}
        </button>
        <p role="status" className="vl-muted">
          {counts !== undefined && counts.sampled_at_ms > 0
            ? t("admin-usage-counted", {
                when: TIME.format(new Date(counts.sampled_at_ms)),
              })
            : ""}
        </p>
      </div>
      {loaded.state === "failed" && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
          <CopyDiagnostic message={loaded.message} />
        </p>
      )}
      {counts !== undefined &&
        (counts.sampled_at_ms > 0 ? (
          <Figures counts={counts} />
        ) : (
          <p>{t("admin-usage-none")}</p>
        ))}
    </>
  );
}

/** Admin → Usage: people, activity, content and storage on this server. */
export function UsagePage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-usage")} intro={t("admin-usage-intro")}>
      {(api) => <Overview api={api} />}
    </AdminScreen>
  );
}
