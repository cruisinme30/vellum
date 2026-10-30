// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps } from "react";
import { Link, useSearchParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { MyWorkSettingsView, MyWorkView, TaskView, WorkApi } from "./api";

import "./work.css";

/** A mention: who wrote it, where, and a button to put it away. */
function MentionRow({
  task,
  onDismiss,
}: {
  readonly task: TaskView;
  readonly onDismiss: (task: TaskView) => void;
}) {
  const t = useT();
  const key = task.issue_key ?? undefined;
  const page = task.doc ?? undefined;
  return (
    <li className="vl-task vl-task-mention">
      <span className="vl-task-title">
        <small className="vl-muted">
          {t("my-work-mentioned-by", { person: task.mentioned_by ?? "" })}
        </small>{" "}
        {task.title}
      </span>
      {key !== undefined && <Link to={`/issues/${encodeURIComponent(key)}`}>{key}</Link>}
      {key === undefined && page !== undefined && (
        <Link to={`/p/${encodeURIComponent(page)}`}>{t("my-work-open-page")}</Link>
      )}
      <button
        type="button"
        className="vl-text-button"
        aria-label={t("my-work-dismiss-label", { title: task.title })}
        onClick={() => {
          onDismiss(task);
        }}
      >
        {t("my-work-dismiss")}
      </button>
    </li>
  );
}

/** One task: its tick box, its title (a link for an issue) and when it is due. */
function TaskRow({
  task,
  onToggle,
  onDismiss,
}: {
  readonly task: TaskView;
  readonly onToggle: (task: TaskView) => void;
  readonly onDismiss: (task: TaskView) => void;
}) {
  const t = useT();
  const key = task.issue_key ?? undefined;
  if (task.kind === "mention") return <MentionRow task={task} onDismiss={onDismiss} />;
  return (
    <li className={task.overdue ? "vl-task vl-task-overdue" : "vl-task"}>
      <label>
        <input
          type="checkbox"
          checked={task.done}
          onChange={() => {
            onToggle(task);
          }}
        />
        <span className="vl-task-title">{task.title}</span>
      </label>
      {key !== undefined && <Link to={`/issues/${encodeURIComponent(key)}`}>{key}</Link>}
      {task.fields.due != undefined && (
        <small className="vl-muted">{t("my-work-due", { date: task.fields.due })}</small>
      )}
    </li>
  );
}

/** The buckets that hold tasks, in the server's order. */
function Buckets({
  view,
  onToggle,
  onDismiss,
}: {
  readonly view: MyWorkView;
  readonly onToggle: (task: TaskView) => void;
  readonly onDismiss: (task: TaskView) => void;
}) {
  const t = useT();
  const sections = view.sections.filter((section) => section.tasks.length > 0);
  if (sections.length === 0) return <p className="vl-muted">{t("my-work-empty")}</p>;
  return (
    <>
      {sections.map((section) => (
        <section key={`${section.bucket}:${section.day ?? section.week_of ?? ""}`}>
          <h2>{section.day ?? t(`my-work-bucket-${section.bucket}`)}</h2>
          <ul className="vl-task-list">
            {section.tasks.map((task) => (
              <TaskRow key={task.id} task={task} onToggle={onToggle} onDismiss={onDismiss} />
            ))}
          </ul>
        </section>
      ))}
    </>
  );
}

// eslint-disable-next-line unicorn/no-null -- the API reads null as "none": the server's zone, no digest
const NONE = null;

/** The browser's IANA time zone, if it reports one. */
function browserZone(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return undefined;
  }
}

/** Every zone the browser knows, for the picker. */
function knownZones(): readonly string[] {
  try {
    return Intl.supportedValuesOf("timeZone");
  } catch {
    return [];
  }
}

/** Time zone, start of day and digest time. */
function SettingsForm({
  api,
  settings,
  onSaved,
}: {
  readonly api: WorkApi;
  readonly settings: MyWorkSettingsView;
  readonly onSaved: () => void;
}) {
  const t = useT();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const here = browserZone();
  const save: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const text = (name: string) => {
      const value = data.get(name);
      return typeof value === "string" ? value.trim() : "";
    };
    setError(undefined);
    setSaved(false);
    api
      .saveSettings({
        time_zone: text("time_zone") === "" ? NONE : text("time_zone"),
        day_starts_at: text("day_starts_at"),
        digest_at: text("digest_at") === "" ? NONE : text("digest_at"),
      })
      .then(
        () => {
          setSaved(true);
          onSaved();
        },
        (error_: unknown) => {
          setError(failureText(error_));
        },
      );
  };
  return (
    <form className="vl-work-settings" onSubmit={save}>
      <p className="vl-muted">
        {t("my-work-settings-zone-now", { zone: settings.effective_time_zone })}
      </p>
      <label>
        {t("my-work-settings-zone")}
        <input
          name="time_zone"
          list="vl-time-zones"
          autoComplete="off"
          defaultValue={settings.time_zone ?? here ?? ""}
          placeholder={t("my-work-settings-zone-server")}
        />
      </label>
      <datalist id="vl-time-zones">
        {knownZones().map((zone) => (
          <option key={zone} value={zone} />
        ))}
      </datalist>
      <label>
        {t("my-work-settings-day")}
        <input name="day_starts_at" type="time" required defaultValue={settings.day_starts_at} />
      </label>
      <label>
        {t("my-work-settings-digest")}
        <input name="digest_at" type="time" defaultValue={settings.digest_at ?? ""} />
      </label>
      <p className="vl-muted">{t("my-work-settings-digest-hint")}</p>
      <button type="submit" className="vl-filled-button">
        {t("my-work-settings-save")}
      </button>
      {saved && <p role="status">{t("my-work-settings-saved")}</p>}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

function Settings({ api, onSaved }: { readonly api: WorkApi; readonly onSaved: () => void }) {
  const t = useT();
  const loaded = useLoaded(() => api.settings(), [api], failureText);
  return (
    <details className="vl-work-settings-panel">
      <summary>{t("my-work-settings")}</summary>
      <Ready loaded={loaded}>
        {(settings) => <SettingsForm api={api} settings={settings} onSaved={onSaved} />}
      </Ready>
    </details>
  );
}

/**
 * Opened from Slack's "Save as task" reply (`?slack_link=CODE`): asks
 * before linking, so a link someone else sent cannot link their Slack
 * account to yours unnoticed.
 */
function SlackLink({ api }: { readonly api: WorkApi }) {
  const t = useT();
  const [parameters, setParameters] = useSearchParams();
  const code = parameters.get("slack_link");
  const [state, setState] = useState<"ask" | "busy" | "done">("ask");
  const [error, setError] = useState<string>();
  const link = api.linkSlack?.bind(api);
  if (code === null || link === undefined) return;
  const close = () => {
    const next = new URLSearchParams(parameters);
    next.delete("slack_link");
    setParameters(next, { replace: true });
  };
  if (state === "done") {
    return (
      <div className="vl-work-banner" role="status">
        <p>{t("my-work-slack-linked")}</p>
        <button type="button" className="vl-text-button" onClick={close}>
          {t("my-work-slack-close")}
        </button>
      </div>
    );
  }
  return (
    <div className="vl-work-banner" role="region" aria-label={t("my-work-slack-title")}>
      <p>{t("my-work-slack-ask")}</p>
      <div className="vl-work-banner-actions">
        <button
          type="button"
          className="vl-filled-button"
          disabled={state === "busy"}
          onClick={() => {
            setState("busy");
            setError(undefined);
            link(code).then(
              () => {
                setState("done");
              },
              (error_: unknown) => {
                setState("ask");
                setError(failureText(error_));
              },
            );
          }}
        >
          {t("my-work-slack-link")}
        </button>
        <button type="button" className="vl-text-button" onClick={close}>
          {t("my-work-slack-cancel")}
        </button>
      </div>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function MyWork({ api }: { readonly api: WorkApi }) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(() => api.myWork(), [api, version], failureText);
  const run = (write: Promise<unknown>) => {
    setError(undefined);
    write.then(
      () => {
        setVersion((v) => v + 1);
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  const add: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const text = new FormData(form).get("text");
    if (typeof text !== "string" || text.trim() === "") return;
    form.reset();
    run(api.addTask(text.trim()));
  };
  return (
    <>
      <SlackLink api={api} />
      <form className="vl-work-add" onSubmit={add}>
        <label>
          {t("my-work-add")}
          <input name="text" autoComplete="off" placeholder={t("my-work-add-hint")} />
        </label>
        <button type="submit" className="vl-filled-button">
          {t("my-work-add-button")}
        </button>
      </form>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <Ready loaded={loaded}>
        {(view) => (
          <Buckets
            view={view}
            onToggle={(task) => {
              run(api.setDone(task.id, !task.done));
            }}
            onDismiss={(task) => {
              run(api.dismissMention(task.id));
            }}
          />
        )}
      </Ready>
      <Settings
        api={api}
        onSaved={() => {
          setVersion((v) => v + 1);
        }}
      />
    </>
  );
}

/** My Work: the signed-in person's tasks and assigned issues, by when they are due. */
export function MyWorkPage() {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("my-work")} className="vl-work">
      {(api) => <MyWork api={api} />}
    </ServerScreen>
  );
}
