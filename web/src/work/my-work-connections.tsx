// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * My Work → Settings (#525, D298): the calendar feed and CalDAV, capture
 * by e-mail and Slack, and imports from other task apps. Secrets are shown
 * once, when issued: the server keeps only their hashes.
 */

import { useId, useState, type ComponentProps, type ReactNode } from "react";
import { Link } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { ConnectionsApi, ConnectionsView, ImportFrom, ImportView } from "./connections-api";
import { previewCount } from "./import-preview";

import "./work.css";

/** Runs a write, keeping its error for the section to show. */
function useRun() {
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const run = <T,>(write: () => Promise<T>, done: (value: T) => void) => {
    setError(undefined);
    setBusy(true);
    write().then(
      (value) => {
        setBusy(false);
        done(value);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return { error, busy, run };
}

function Failure({ error }: { readonly error: string | undefined }) {
  if (error === undefined) return;
  return (
    <p className="vl-field-error" role="alert">
      {error}
    </p>
  );
}

/** A secret or URL to copy, in a read-only field with a Copy button. */
function Copyable({ label, value }: { readonly label: string; readonly value: string }) {
  const t = useT();
  const id = useId();
  const [copied, setCopied] = useState(false);
  return (
    <div className="vl-copyable">
      <label htmlFor={id}>{label}</label>
      <div>
        <input
          id={id}
          readOnly
          value={value}
          onFocus={(event) => {
            event.currentTarget.select();
          }}
        />
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            void navigator.clipboard.writeText(value).then(
              () => {
                setCopied(true);
              },
              () => {
                // The field is selected on focus, so copying by hand still works.
              },
            );
          }}
        >
          {t("connections-copy")}
        </button>
      </div>
      {copied && <p role="status">{t("connections-copied")}</p>}
    </div>
  );
}

function Section({
  title,
  intro,
  children,
}: {
  readonly title: string;
  readonly intro: string;
  readonly children: ReactNode;
}) {
  const id = useId();
  return (
    <section className="vl-connections-section" aria-labelledby={id}>
      <h2 id={id}>{title}</h2>
      <p className="vl-muted">{intro}</p>
      {children}
    </section>
  );
}

/** The feed URL, CalDAV URL and an app password for CalDAV apps. */
function Calendar({ api, view }: { readonly api: ConnectionsApi; readonly view: ConnectionsView }) {
  const t = useT();
  const { error, busy, run } = useRun();
  const [feed, setFeed] = useState<string>();
  const [off, setOff] = useState(false);
  const [password, setPassword] = useState<string>();
  return (
    <Section title={t("connections-calendar")} intro={t("connections-calendar-intro")}>
      {feed !== undefined && <Copyable label={t("connections-feed-url")} value={feed} />}
      {off && <p role="status">{t("connections-feed-off")}</p>}
      <div className="vl-connections-actions">
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy}
          onClick={() => {
            run(api.rotateFeed, (url) => {
              setOff(false);
              setFeed(url);
            });
          }}
        >
          {t("connections-feed-new")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            run(api.revokeFeed, () => {
              setFeed(undefined);
              setOff(true);
            });
          }}
        >
          {t("connections-feed-revoke")}
        </button>
      </div>
      <p className="vl-muted">{t("connections-feed-hint")}</p>
      <h3>{t("connections-caldav")}</h3>
      <Copyable label={t("connections-caldav-url")} value={view.caldav_url} />
      <ul className="vl-connections-notes">
        <li>{t("connections-caldav-apple")}</li>
        <li>{t("connections-caldav-thunderbird")}</li>
      </ul>
      {password === undefined ? (
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            run(() => api.appPassword(t("connections-caldav-token-name")), setPassword);
          }}
        >
          {t("connections-caldav-password")}
        </button>
      ) : (
        <Copyable label={t("connections-caldav-password-label")} value={password} />
      )}
      <Failure error={error} />
    </Section>
  );
}

/** The capture address and Slack's status. */
function Capture({
  api,
  view,
  onChanged,
}: {
  readonly api: ConnectionsApi;
  readonly view: ConnectionsView;
  readonly onChanged: () => void;
}) {
  const t = useT();
  const { error, busy, run } = useRun();
  const [address, setAddress] = useState<string>();
  const [off, setOff] = useState(false);
  return (
    <Section title={t("connections-capture")} intro={t("connections-capture-intro")}>
      <h3>{t("connections-email")}</h3>
      {view.email ? (
        <>
          {address !== undefined && (
            <Copyable label={t("connections-email-address")} value={address} />
          )}
          {off && <p role="status">{t("connections-email-off")}</p>}
          <div className="vl-connections-actions">
            <button
              type="button"
              className="vl-filled-button"
              disabled={busy}
              onClick={() => {
                run(api.rotateCapture, (value) => {
                  setOff(false);
                  setAddress(value);
                });
              }}
            >
              {t("connections-email-new")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              disabled={busy}
              onClick={() => {
                run(api.revokeCapture, () => {
                  setAddress(undefined);
                  setOff(true);
                });
              }}
            >
              {t("connections-email-revoke")}
            </button>
          </div>
        </>
      ) : (
        <p className="vl-muted">{t("connections-email-unavailable")}</p>
      )}
      <h3>{t("connections-slack")}</h3>
      {!view.slack && <p className="vl-muted">{t("connections-slack-unavailable")}</p>}
      {view.slack && !view.slack_linked && <p className="vl-muted">{t("connections-slack-how")}</p>}
      {view.slack && view.slack_linked && (
        <>
          <p>{t("connections-slack-linked")}</p>
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            onClick={() => {
              run(api.unlinkSlack, onChanged);
            }}
          >
            {t("connections-slack-unlink")}
          </button>
        </>
      )}
      <Failure error={error} />
    </Section>
  );
}

const SOURCES: readonly { readonly from: ImportFrom; readonly accept: string }[] = [
  { from: "google_tasks", accept: ".json,application/json" },
  { from: "todoist", accept: ".csv,text/csv" },
  { from: "things", accept: ".json,application/json" },
];

/** Upload an export, see how many tasks it holds, import it. */
function Import({ api }: { readonly api: ConnectionsApi }) {
  const t = useT();
  const { error, busy, run } = useRun();
  const [from, setFrom] = useState<ImportFrom>("google_tasks");
  const [file, setFile] = useState<{ name: string; content: string }>();
  const [result, setResult] = useState<ImportView>();
  const count = file === undefined ? undefined : previewCount(from, file.content);
  const accept = SOURCES.find((source) => source.from === from)?.accept;
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    if (file === undefined) return;
    const project = new FormData(event.currentTarget).get("project");
    run(
      () => api.importTasks(from, file.content, typeof project === "string" ? project : undefined),
      setResult,
    );
  };
  return (
    <Section title={t("connections-import")} intro={t("connections-import-intro")}>
      <form className="vl-connections-form" onSubmit={submit}>
        <label>
          {t("connections-import-from")}
          <select
            value={from}
            onChange={(event) => {
              setFrom(event.target.value as ImportFrom);
              setResult(undefined);
            }}
          >
            {SOURCES.map((source) => (
              <option key={source.from} value={source.from}>
                {t(`connections-import-${source.from}`)}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("connections-import-file")}
          <input
            type="file"
            accept={accept}
            onChange={(event) => {
              const picked = event.target.files?.[0];
              setResult(undefined);
              if (picked === undefined) {
                setFile(undefined);
                return;
              }
              void picked.text().then((content) => {
                setFile({ name: picked.name, content });
              });
            }}
          />
        </label>
        {from === "todoist" && (
          <label>
            {t("connections-import-project")}
            <input name="project" autoComplete="off" placeholder="Todoist" />
          </label>
        )}
        <button
          type="submit"
          className="vl-filled-button"
          disabled={busy || file === undefined || count === undefined || count === 0}
        >
          {t("connections-import-button")}
        </button>
      </form>
      {file !== undefined && (
        <p role="status">
          {count === undefined
            ? t("connections-import-unreadable", { file: file.name })
            : t("connections-import-preview", { count, file: file.name })}
        </p>
      )}
      {result !== undefined && (
        <div role="status">
          <p>{t("connections-import-done")}</p>
          <ul>
            {result.lists.map((list) => (
              <li key={list.doc}>
                {t("connections-import-list", { list: list.title, count: list.added })}
              </li>
            ))}
          </ul>
          <Link to="/my-work">{t("connections-back")}</Link>
        </div>
      )}
      <Failure error={error} />
    </Section>
  );
}

function Connections({ api }: { readonly api: ConnectionsApi }) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(() => api.connections(), [api, version], failureText);
  return (
    <>
      <p>
        <Link to="/my-work">{t("connections-back")}</Link>
      </p>
      <Ready loaded={loaded}>
        {(view) => (
          <>
            <Calendar api={api} view={view} />
            <Capture
              api={api}
              view={view}
              onChanged={() => {
                setVersion((v) => v + 1);
              }}
            />
          </>
        )}
      </Ready>
      <Import api={api} />
    </>
  );
}

/** My Work settings: calendars, capture and imports. */
export function MyWorkConnectionsPage() {
  const t = useT();
  return (
    <ServerScreen api="connections" title={t("connections-title")} className="vl-work">
      {(api) => <Connections api={api} />}
    </ServerScreen>
  );
}
