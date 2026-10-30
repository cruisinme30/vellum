// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";

import { formText, type AdminConsoleApi, type AuditRecord, type ChainReport } from "./api";
import { AdminScreen } from "./context";

/** Records per page. */
const PAGE = 50;

const TIME = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

interface Filters {
  readonly action: string;
  readonly user: string;
}

function AuditRow({ record }: { readonly record: AuditRecord }) {
  const t = useT();
  const { event } = record;
  const who = event.actor.user_id ?? event.actor.kind;
  return (
    <tr className={event.outcome === "success" ? undefined : "vl-admin-row-problem"}>
      <td>{record.seq}</td>
      <td>
        <time dateTime={record.at}>{TIME.format(new Date(record.at))}</time>
      </td>
      <td>{who}</td>
      <th scope="row">{event.action}</th>
      <td>{event.target ?? "–"}</td>
      <td>{t(`admin-outcome-${event.outcome}`)}</td>
      <td>{event.request_id ?? "–"}</td>
    </tr>
  );
}

function AuditTable({ records }: { readonly records: readonly AuditRecord[] }) {
  const t = useT();
  if (records.length === 0) {
    return <p>{t("admin-audit-empty")}</p>;
  }
  return (
    <div className="vl-admin-scroll">
      <table className="vl-admin-table">
        <caption>{t("admin-audit-caption")}</caption>
        <thead>
          <tr>
            <th scope="col">#</th>
            <th scope="col">{t("admin-audit-when")}</th>
            <th scope="col">{t("admin-audit-who")}</th>
            <th scope="col">{t("admin-audit-action")}</th>
            <th scope="col">{t("admin-audit-target")}</th>
            <th scope="col">{t("admin-audit-outcome")}</th>
            <th scope="col">{t("admin-audit-request")}</th>
          </tr>
        </thead>
        <tbody>
          {records.map((record) => (
            <AuditRow key={record.seq} record={record} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Verify the hash chain and export the log. */
function ChainTools({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const [report, setReport] = useState<ChainReport>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [download, setDownload] = useState<string>();

  const run = (task: () => Promise<void>) => {
    setBusy(true);
    setError(undefined);
    task()
      .catch((error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <div className="vl-admin-actions">
      <button
        type="button"
        className="vl-text-button"
        disabled={busy}
        onClick={() => {
          run(async () => {
            setReport(await api.verifyAudit());
          });
        }}
      >
        {t("admin-audit-verify")}
      </button>
      <button
        type="button"
        className="vl-text-button"
        disabled={busy}
        onClick={() => {
          run(async () => {
            const jsonl = await api.exportAudit();
            if (download !== undefined) URL.revokeObjectURL(download);
            setDownload(URL.createObjectURL(new Blob([jsonl], { type: "application/jsonl" })));
          });
        }}
      >
        {t("admin-audit-export")}
      </button>
      {download !== undefined && (
        <a href={download} download="vellum-audit.jsonl">
          {t("admin-audit-download")}
        </a>
      )}
      <div role="status">
        {report !== undefined && (
          <p className={report.broken_at ? "vl-field-error" : undefined}>
            {report.broken_at
              ? t("admin-audit-broken", {
                  seq: report.broken_at.seq,
                  reason: report.broken_at.reason,
                })
              : t("admin-audit-intact", { records: report.records })}
          </p>
        )}
      </div>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
          <CopyDiagnostic message={error} />
        </p>
      )}
    </div>
  );
}

/** The records for the filters, a page at a time; "Older" moves back. */
function AuditRecords({
  api,
  filters,
}: {
  readonly api: AdminConsoleApi;
  readonly filters: Filters;
}) {
  const t = useT();
  const [pages, setPages] = useState<readonly number[]>([]);
  const before = pages.at(-1);
  const loaded = useLoaded(
    () =>
      api.audit({
        ...(filters.action === "" ? {} : { action: filters.action }),
        ...(filters.user === "" ? {} : { user_id: filters.user }),
        ...(before === undefined ? {} : { before }),
        limit: PAGE,
      }),
    [api, filters, before],
    failureText,
  );
  if (loaded.state === "loading") {
    return <p className="vl-muted">{t("admin-loading")}</p>;
  }
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
        <CopyDiagnostic message={loaded.message} />
      </p>
    );
  }
  const records = loaded.value;
  const oldest = records.at(-1)?.seq;
  return (
    <>
      <AuditTable records={records} />
      <div className="vl-admin-actions">
        {pages.length > 0 && (
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setPages(pages.slice(0, -1));
            }}
          >
            {t("admin-newer")}
          </button>
        )}
        {records.length === PAGE && oldest !== undefined && (
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setPages([...pages, oldest]);
            }}
          >
            {t("admin-older")}
          </button>
        )}
      </div>
    </>
  );
}

function AuditFilters({ onApply }: { readonly onApply: (filters: Filters) => void }) {
  const t = useT();
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    onApply({ action: formText(data, "action"), user: formText(data, "user") });
  };
  return (
    <form className="vl-admin-filters" onSubmit={submit}>
      <label>
        {t("admin-audit-action")}
        <input name="action" placeholder="admin.settings_update" />
      </label>
      <label>
        {t("admin-audit-user")}
        <input name="user" />
      </label>
      <button type="submit" className="vl-text-button">
        {t("admin-filter")}
      </button>
    </form>
  );
}

/** Admin → Audit: the tamper-evident log of who did what, newest first. */
export function AuditPage() {
  const t = useT();
  const [filters, setFilters] = useState<Filters>({ action: "", user: "" });
  return (
    <AdminScreen help="admin.audit" title={t("admin-audit")} intro={t("admin-audit-intro")}>
      {(api) => (
        <>
          <ChainTools api={api} />
          <AuditFilters onApply={setFilters} />
          <AuditRecords key={`${filters.action}\n${filters.user}`} api={api} filters={filters} />
        </>
      )}
    </AdminScreen>
  );
}
