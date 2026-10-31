// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Time-blocking (R-MW-9): a task gets a block of time, and when Google
 * Calendar is connected each block is an event in a "Vellum" calendar,
 * synced both ways every few minutes (ADR 0018, #162).
 */

import { useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { GoogleView, TaskView, WorkApi } from "./api";
import {
  blockEnd,
  formatBlock,
  MAX_BLOCK_MINUTES,
  parseBlock,
} from "./schedule";

/** The block a task has, as "15:00–16:30, 2026-11-03". */
export function BlockLabel({ task }: { readonly task: TaskView }) {
  const t = useT();
  const block = parseBlock(task.fields.block);
  if (block === undefined) return;
  return (
    <small className="vl-muted vl-task-block">
      {t("my-work-block-at", {
        day: block.start.slice(0, 10),
        from: block.start.slice(11, 16),
        to: blockEnd(block),
      })}
    </small>
  );
}

/** Sets or clears a task's time block. */
export function BlockForm({
  api,
  task,
  today,
  onDone,
}: {
  readonly api: WorkApi;
  readonly task: TaskView;
  readonly today: string;
  readonly onDone: () => void;
}) {
  const t = useT();
  const [error, setError] = useState<string>();
  const current = parseBlock(task.fields.block);
  const save = (block: string) => {
    setError(undefined);
    api.update(task.id, { block }).then(onDone, (error_: unknown) => {
      setError(failureText(error_));
    });
  };
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const start = data.get("start");
    const block = formatBlock(
      typeof start === "string" ? start : "",
      Number(data.get("minutes")),
    );
    if (block === undefined) {
      setError(t("my-work-block-invalid"));
      return;
    }
    save(block);
  };
  return (
    <form
      className="vl-block-form"
      aria-label={t("my-work-block-for", { title: task.title })}
      onSubmit={submit}
    >
      <label>
        {t("my-work-block-start")}
        <input
          name="start"
          type="datetime-local"
          required
          defaultValue={current?.start ?? `${task.fields.start ?? today}T09:00`}
        />
      </label>
      <label>
        {t("my-work-block-minutes")}
        <input
          name="minutes"
          type="number"
          min={5}
          max={MAX_BLOCK_MINUTES}
          step={5}
          required
          defaultValue={current?.minutes ?? 60}
        />
      </label>
      <button type="submit" className="vl-filled-button">
        {t("my-work-block-save")}
      </button>
      {current !== undefined && (
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            save("");
          }}
        >
          {t("my-work-block-clear")}
        </button>
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** Google Calendar's state and its buttons. */
function Google({
  api,
  status,
  onStatus,
  onSynced,
}: {
  readonly api: WorkApi;
  readonly status: GoogleView;
  /** After connecting or disconnecting: the status loads again. */
  readonly onStatus: () => void;
  /** After a sync: the tasks load again; the note stays. */
  readonly onSynced: () => void;
}) {
  const t = useT();
  const [note, setNote] = useState<string>();
  const [error, setError] = useState<string>();
  const run = <T,>(write: Promise<T>, after: (value: T) => void) => {
    setError(undefined);
    setNote(undefined);
    write.then(after, (error_: unknown) => {
      setError(failureText(error_));
    });
  };
  if (!status.configured)
    return <p className="vl-muted">{t("my-work-google-off")}</p>;
  return (
    <>
      <p className="vl-muted">
        {t(
          status.connected
            ? "my-work-google-connected"
            : "my-work-google-not-connected",
        )}
      </p>
      <div className="vl-quick-actions">
        {status.connected ? (
          <>
            <button
              type="button"
              className="vl-filled-button"
              onClick={() => {
                run(api.syncGoogle(), (synced) => {
                  setNote(
                    t("my-work-google-synced", {
                      changed:
                        synced.created +
                        synced.updated +
                        synced.moved +
                        synced.deleted,
                    }),
                  );
                  onSynced();
                });
              }}
            >
              {t("my-work-google-sync")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                run(api.disconnectGoogle(), onStatus);
              }}
            >
              {t("my-work-google-disconnect")}
            </button>
          </>
        ) : (
          <button
            type="button"
            className="vl-filled-button"
            onClick={() => {
              run(api.connectGoogle(), (consent) => {
                // Google's consent page; it comes back to
                // /connect/google/callback.
                globalThis.location.assign(consent);
              });
            }}
          >
            {t("my-work-google-connect")}
          </button>
        )}
      </div>
      {note !== undefined && <p role="status">{note}</p>}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </>
  );
}

/** The Calendar panel under My Work. */
export function CalendarPanel({
  api,
  onChanged,
}: {
  readonly api: WorkApi;
  readonly onChanged: () => void;
}) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const loaded = useLoaded(() => api.google(), [api, version], failureText);
  return (
    <details className="vl-work-panel">
      <summary>{t("my-work-google")}</summary>
      <p className="vl-muted">{t("my-work-google-intro")}</p>
      <Ready loaded={loaded}>
        {(status) => (
          <Google
            api={api}
            status={status}
            onStatus={() => {
              setVersion((v) => v + 1);
              onChanged();
            }}
            onSynced={onChanged}
          />
        )}
      </Ready>
    </details>
  );
}
