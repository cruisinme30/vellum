// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps } from "react";
import { Link } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { MyWorkView, TaskView, WorkApi } from "./api";

import "./work.css";

/** One task: its tick box, its title (a link for an issue) and when it is due. */
function TaskRow({
  task,
  onToggle,
}: {
  readonly task: TaskView;
  readonly onToggle: (task: TaskView) => void;
}) {
  const t = useT();
  const key = task.issue_key ?? undefined;
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
}: {
  readonly view: MyWorkView;
  readonly onToggle: (task: TaskView) => void;
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
              <TaskRow key={task.id} task={task} onToggle={onToggle} />
            ))}
          </ul>
        </section>
      ))}
    </>
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
          />
        )}
      </Ready>
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
