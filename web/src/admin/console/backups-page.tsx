// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { formatSize } from "../../data/format";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import {
  failureText,
  type AdminConsoleApi,
  type AdminState,
  type BackupFile,
  type DrillReport,
} from "./api";
import { AdminScreen } from "./context";

const TIME = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

const when = (iso: string) => TIME.format(new Date(iso));

function DrillLine({ drill }: { readonly drill: DrillReport }) {
  const t = useT();
  const arguments_ = { when: when(drill.ran_at), backup: drill.backup, files: drill.files };
  return drill.passed ? (
    <p>{t("admin-backups-drill-passed", arguments_)}</p>
  ) : (
    <p className="vl-field-error">
      {t("admin-backups-drill-failed", { ...arguments_, error: drill.error ?? "" })}
    </p>
  );
}

/** When backups, drills and telemetry last ran. */
function StateSummary({ state }: { readonly state: AdminState }) {
  const t = useT();
  return (
    <dl className="vl-admin-state">
      <div>
        <dt>{t("admin-backups-last")}</dt>
        <dd>
          {state.last_backup
            ? t("admin-backups-last-detail", {
                when: when(state.last_backup.created_at),
                files: state.last_backup.files,
                size: formatSize(state.last_backup.size),
              })
            : t("admin-backups-never")}
        </dd>
      </div>
      <div>
        <dt>{t("admin-backups-drill")}</dt>
        <dd>
          {state.last_drill ? <DrillLine drill={state.last_drill} /> : t("admin-backups-never")}
        </dd>
      </div>
      <div>
        <dt>{t("admin-backups-telemetry")}</dt>
        <dd>
          {typeof state.last_telemetry_ms === "number"
            ? TIME.format(new Date(state.last_telemetry_ms))
            : t("admin-backups-never")}
        </dd>
      </div>
    </dl>
  );
}

function BackupTable({ backups }: { readonly backups: readonly BackupFile[] }) {
  const t = useT();
  if (backups.length === 0) {
    return <p>{t("admin-backups-empty")}</p>;
  }
  return (
    <div className="vl-admin-scroll">
      <table className="vl-admin-table">
        <caption>{t("admin-backups-caption")}</caption>
        <thead>
          <tr>
            <th scope="col">{t("admin-backups-file")}</th>
            <th scope="col">{t("admin-backups-size")}</th>
          </tr>
        </thead>
        <tbody>
          {backups.map((backup) => (
            <tr key={backup.name}>
              <th scope="row">{backup.name}</th>
              <td>{formatSize(backup.size)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Overview({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState<"backup" | "drill">();
  const [notice, setNotice] = useState<{ ok: boolean; message: string }>();
  const loaded = useLoaded(
    async () => {
      const [state, backups] = await Promise.all([api.state(), api.backups()]);
      return { state, backups };
    },
    [api, reload],
    failureText,
  );

  const act = (kind: "backup" | "drill") => {
    setBusy(kind);
    setNotice(undefined);
    const task =
      kind === "backup"
        ? api
            .backUpNow()
            .then((info) =>
              t("admin-backups-made", { name: info.name, size: formatSize(info.size) }),
            )
        : api
            .runDrill()
            .then((drill) =>
              drill.passed
                ? t("admin-backups-drill-ok", { files: drill.files })
                : t("admin-backups-drill-bad", { error: drill.error ?? "" }),
            );
    task
      .then(
        (message) => {
          setNotice({ ok: true, message });
        },
        (error: unknown) => {
          setNotice({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(undefined);
        setReload((n) => n + 1);
      });
  };

  return (
    <>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy !== undefined}
          onClick={() => {
            act("backup");
          }}
        >
          {busy === "backup" ? t("admin-backups-running") : t("admin-backups-now")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy !== undefined}
          onClick={() => {
            act("drill");
          }}
        >
          {busy === "drill" ? t("admin-backups-drilling") : t("admin-backups-run-drill")}
        </button>
        <div role="status">{notice?.ok === true && <p>{notice.message}</p>}</div>
      </div>
      {notice?.ok === false && (
        <p className="vl-field-error" role="alert">
          {notice.message}
        </p>
      )}
      {loaded.state === "loading" && <p className="vl-muted">{t("admin-loading")}</p>}
      {loaded.state === "failed" && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
        </p>
      )}
      {loaded.state === "ready" && (
        <>
          <StateSummary state={loaded.value.state} />
          <BackupTable backups={loaded.value.backups} />
        </>
      )}
    </>
  );
}

/** Admin → Backups: the last backup and drill, the backups kept, and both on demand. */
export function BackupsPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-backups")} intro={t("admin-backups-intro")}>
      {(api) => <Overview api={api} />}
    </AdminScreen>
  );
}
