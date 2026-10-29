// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useEffect, useState, type DragEvent } from "react";
import { Link, useSearchParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { MyWorkView, TaskView, WorkApi } from "./api";
import { BlockForm, BlockLabel, CalendarPanel } from "./my-work-calendar";
import { ListsPanel } from "./my-work-lists";
import { Settings } from "./my-work-settings";
import { QuickAddForm, TASKS_CHANGED } from "./quick-add";
import { isMovable, moveTargets, sectionTarget } from "./schedule";

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

/** The drag payload type for a task, so drops from elsewhere are ignored. */
const TASK_DRAG = "application/x-vellum-task";

/** What a row can do; each is wired by the page. */
interface RowActions {
  readonly onToggle: (task: TaskView) => void;
  readonly onDismiss: (task: TaskView) => void;
  readonly onMove: (task: TaskView, to: string) => void;
  readonly onDragging: (task?: TaskView) => void;
}

/** The keyboard and screen-reader way to reschedule: a Move menu. */
function MoveMenu({
  task,
  today,
  onMove,
}: {
  readonly task: TaskView;
  readonly today: string;
  readonly onMove: (task: TaskView, to: string) => void;
}) {
  const t = useT();
  return (
    <select
      className="vl-task-move"
      aria-label={t("my-work-move-label", { title: task.title })}
      value=""
      onChange={(event) => {
        if (event.target.value !== "") onMove(task, event.target.value);
      }}
    >
      <option value="">{t("my-work-move")}</option>
      {moveTargets(today).map((target) => (
        <option key={target.to} value={target.to}>
          {"key" in target.label ? t(target.label.key) : target.label.day}
        </option>
      ))}
    </select>
  );
}

/** One task: its tick box, its title (a link for an issue) and when it is due. */
function TaskRow({
  api,
  task,
  today,
  actions,
  onChanged,
}: {
  readonly api: WorkApi;
  readonly task: TaskView;
  readonly today: string;
  readonly actions: RowActions;
  readonly onChanged: () => void;
}) {
  const t = useT();
  const [blocking, setBlocking] = useState(false);
  const key = task.issue_key ?? undefined;
  if (task.kind === "mention") return <MentionRow task={task} onDismiss={actions.onDismiss} />;
  const movable = isMovable(task) && !task.done;
  return (
    // Dragging is the pointer shortcut; the Move menu is the keyboard way (R-MW-1).
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <li
      className={task.overdue ? "vl-task vl-task-overdue" : "vl-task"}
      draggable={movable}
      onDragStart={(event) => {
        if (!movable) return;
        event.dataTransfer.setData(TASK_DRAG, task.id);
        event.dataTransfer.effectAllowed = "move";
        actions.onDragging(task);
      }}
      onDragEnd={() => {
        actions.onDragging();
      }}
    >
      <label>
        <input
          type="checkbox"
          checked={task.done}
          onChange={() => {
            actions.onToggle(task);
          }}
        />
        <span className="vl-task-title">{task.title}</span>
      </label>
      {key !== undefined && <Link to={`/issues/${encodeURIComponent(key)}`}>{key}</Link>}
      {task.list != undefined && <small className="vl-muted">#{task.list}</small>}
      {task.fields.due != undefined && (
        <small className="vl-muted">{t("my-work-due", { date: task.fields.due })}</small>
      )}
      <BlockLabel task={task} />
      {movable && (
        <>
          <MoveMenu task={task} today={today} onMove={actions.onMove} />
          <button
            type="button"
            className="vl-text-button"
            aria-expanded={blocking}
            aria-label={t("my-work-block-for", { title: task.title })}
            onClick={() => {
              setBlocking(!blocking);
            }}
          >
            {t("my-work-block")}
          </button>
        </>
      )}
      {blocking && (
        <BlockForm
          api={api}
          task={task}
          today={today}
          onDone={() => {
            setBlocking(false);
            onChanged();
          }}
        />
      )}
    </li>
  );
}

/** Takes a dropped task, if the drag carries one. */
function dropProperties(onDrop: (task: string) => void) {
  return {
    onDragOver: (event: DragEvent) => {
      if (event.dataTransfer.types.includes(TASK_DRAG)) {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
      }
    },
    onDrop: (event: DragEvent) => {
      const task = event.dataTransfer.getData(TASK_DRAG);
      if (task === "") return;
      event.preventDefault();
      onDrop(task);
    },
  };
}

/** While a task is dragged: every place it can go, including buckets with nothing in them. */
function DropBar({
  today,
  onDrop,
}: {
  readonly today: string;
  readonly onDrop: (task: string, to: string) => void;
}) {
  const t = useT();
  return (
    <ul className="vl-drop-bar" aria-label={t("my-work-drop-here")}>
      {moveTargets(today).map((target) => (
        <li
          key={target.to}
          {...dropProperties((task) => {
            onDrop(task, target.to);
          })}
        >
          {"key" in target.label ? t(target.label.key) : target.label.day}
        </li>
      ))}
    </ul>
  );
}

/** The buckets that hold tasks, in the server's order; each takes dropped tasks. */
function Buckets({
  api,
  view,
  actions,
  onChanged,
}: {
  readonly api: WorkApi;
  readonly view: MyWorkView;
  readonly actions: RowActions;
  readonly onChanged: () => void;
}) {
  const t = useT();
  const [dragging, setDragging] = useState<TaskView>();
  const rowActions: RowActions = {
    ...actions,
    onDragging: (task) => {
      setDragging(task);
      actions.onDragging(task);
    },
  };
  const moveById = (id: string, to: string) => {
    const task = view.sections.flatMap((section) => section.tasks).find((each) => each.id === id);
    setDragging(undefined);
    if (task !== undefined) actions.onMove(task, to);
  };
  const sections = view.sections.filter((section) => section.tasks.length > 0);
  if (sections.length === 0) return <p className="vl-muted">{t("my-work-empty")}</p>;
  return (
    <>
      {dragging !== undefined && <DropBar today={view.today} onDrop={moveById} />}
      {sections.map((section) => {
        const target = sectionTarget(section);
        return (
          <section
            key={`${section.bucket}:${section.day ?? section.week_of ?? ""}`}
            className={
              dragging !== undefined && target !== undefined ? "vl-drop-target" : undefined
            }
            {...(target !== undefined &&
              dropProperties((task) => {
                moveById(task, target);
              }))}
          >
            <h2>{section.day ?? t(`my-work-bucket-${section.bucket}`)}</h2>
            <ul className="vl-task-list">
              {section.tasks.map((task) => (
                <TaskRow
                  key={task.id}
                  api={api}
                  task={task}
                  today={view.today}
                  actions={rowActions}
                  onChanged={onChanged}
                />
              ))}
            </ul>
          </section>
        );
      })}
    </>
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
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(() => api.myWork(), [api, version], failureText);
  const reload = useCallback(() => {
    setVersion((v) => v + 1);
  }, []);
  useEffect(() => {
    globalThis.addEventListener(TASKS_CHANGED, reload);
    return () => {
      globalThis.removeEventListener(TASKS_CHANGED, reload);
    };
  }, [reload]);
  const run = (write: Promise<unknown>) => {
    setError(undefined);
    write.then(reload, (error_: unknown) => {
      setError(failureText(error_));
    });
  };
  const actions: RowActions = {
    onToggle: (task) => {
      run(api.setDone(task.id, !task.done));
    },
    onDismiss: (task) => {
      run(api.dismissMention(task.id));
    },
    onMove: (task, to) => {
      run(api.move(task.id, to));
    },
    onDragging: () => {
      setError(undefined);
    },
  };
  return (
    <>
      <SlackLink api={api} />
      {/* QuickAddForm tells the page through TASKS_CHANGED, like the Q dialog. */}
      <QuickAddForm
        api={api}
        onAdded={() => {
          // The reload comes from the TASKS_CHANGED event.
        }}
      />
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <Ready loaded={loaded}>
        {(view) => <Buckets api={api} view={view} actions={actions} onChanged={reload} />}
      </Ready>
      <ListsPanel api={api} onChanged={reload} />
      <CalendarPanel api={api} onChanged={reload} />
      <Settings api={api} onSaved={reload} />
    </>
  );
}

/** My Work: the signed-in person's tasks and assigned issues, by when they are due. */
export function MyWorkPage() {
  const t = useT();
  return (
    <ServerScreen api="work" help="mywork.buckets" title={t("my-work")} className="vl-work">
      {(api) => <MyWork api={api} />}
    </ServerScreen>
  );
}
