// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Quick-add (R-MW-3): one line of English becomes a task. As the person
 * types, the server's parser marks each phrase it understood ("tomorrow
 * 3pm", "every weekday", `#list @label p1 +person`), so they see what the
 * line will do before pressing Enter. `Q` opens it anywhere; `T` on a
 * selection in a page opens it with the selected text and a link back.
 */

import {
  createContext,
  use,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";

import { failureText } from "../data/failure";
import { useServerApis } from "../data/server-apis";
import { useT } from "../i18n/i18n";

import type { ListView, ParseView, WorkApi } from "./api";

import "./work.css";

/** How long typing pauses before the line is parsed again. */
const PARSE_DELAY_MS = 120;

/**
 * The event My Work listens for to load again after a task is added elsewhere.
 */
export const TASKS_CHANGED = "vellum:tasks-changed";

/** Tells open task views that the tasks changed. */
function announceTasksChanged() {
  globalThis.dispatchEvent(new Event(TASKS_CHANGED));
}

/** A run of the typed line: plain text, or a phrase the parser understood. */
interface Run {
  readonly text: string;
  readonly kind?: ParseView["highlights"][number]["kind"];
}

/**
 * Splits `text` at the parser's highlights (UTF-16 offsets, as JavaScript
 * indexes strings); joining the runs gives `text` back.
 */
function runsOf(text: string, parsed: ParseView | undefined): Run[] {
  const out: Run[] = [];
  let at = 0;
  const spans = (parsed?.highlights ?? [])
    .filter(
      (span) =>
        span.start >= 0 && span.end <= text.length && span.start < span.end,
    )
    .toSorted((a, b) => a.start - b.start);
  for (const span of spans) {
    if (span.start < at) continue;
    if (span.start > at) out.push({ text: text.slice(at, span.start) });
    out.push({ text: text.slice(span.start, span.end), kind: span.kind });
    at = span.end;
  }
  if (at < text.length) out.push({ text: text.slice(at) });
  return out;
}

/**
 * The parse of `text`, a short pause after the last keystroke; stale answers
 * are dropped.
 */
function useParsed(api: WorkApi, text: string): ParseView | undefined {
  const [answer, setAnswer] = useState<{ text: string; parsed: ParseView }>();
  useEffect(() => {
    if (text.trim() === "") return;
    let current = true;
    const timer = setTimeout(() => {
      api.parse(text).then(
        (parsed) => {
          if (current) setAnswer({ text, parsed });
        },
        () => {
          // Highlighting is a hint: the task is still added without it.
        },
      );
    }, PARSE_DELAY_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [api, text]);
  return answer?.text === text ? answer.parsed : undefined;
}

/** What the parsed line will set, as short labelled facts. */
function Facts({ parsed }: { readonly parsed: ParseView }) {
  const t = useT();
  const { fields } = parsed;
  const facts: [string, string][] = [];
  if (fields.due != undefined)
    facts.push(["due", t("quick-add-due", { when: fields.due })]);
  if (fields.start != undefined)
    facts.push(["start", t("quick-add-start", { when: fields.start })]);
  if (fields.when != undefined)
    facts.push(["when", t(`my-work-bucket-${fields.when}`)]);
  if (fields.rrule != undefined) {
    const repeats =
      fields.recur_from === "completion"
        ? "quick-add-repeats-done"
        : "quick-add-repeats";
    facts.push(["recurrence", t(repeats, { rule: fields.rrule })]);
  }
  if (fields.priority != undefined) {
    facts.push([
      "priority",
      t("quick-add-priority", { priority: fields.priority }),
    ]);
  }
  if (parsed.list != undefined)
    facts.push(["list", t("quick-add-list", { list: parsed.list })]);
  for (const label of fields.labels) facts.push(["label", `@${label}`]);
  for (const person of parsed.assignees) facts.push(["person", `+${person}`]);
  for (const remind of fields.remind) {
    facts.push(["reminder", t("quick-add-remind", { when: remind })]);
  }
  if (facts.length === 0) return;
  return (
    <ul className="vl-quick-facts" aria-label={t("quick-add-understood")}>
      {facts.map(([kind, text]) => (
        <li key={`${kind}:${text}`} data-kind={kind}>
          {text}
        </li>
      ))}
    </ul>
  );
}

/** Props of {@link QuickAddForm}. */
export interface QuickAddFormProperties {
  readonly api: WorkApi;
  /** Text to start with, such as a selection in a page. */
  readonly initial?: string;
  /**
   * Added to the new task's title as written, unparsed: a link back to the
   * page.
   */
  readonly suffix?: string | undefined;
  /** Puts the cursor in the line when it opens, as the dialog does. */
  readonly focusOnMount?: boolean;
  /** Called with the new task's title once it is added. */
  readonly onAdded: (title: string) => void;
  /** Extra buttons, such as Cancel in the dialog. */
  readonly children?: ReactNode;
}

/**
 * The quick-add box: the line, its highlighted parse, a list picker and
 * Add. The highlighted copy sits under the field rather than inside it, so
 * the field stays a plain text input for screen readers and IMEs.
 */
export function QuickAddForm({
  api,
  initial = "",
  suffix,
  focusOnMount = false,
  onAdded,
  children,
}: QuickAddFormProperties) {
  const t = useT();
  const id = useId();
  const [text, setText] = useState(initial);
  const [list, setList] = useState("");
  const [lists, setLists] = useState<readonly ListView[]>([]);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const parsed = useParsed(api, text);

  useEffect(() => {
    let current = true;
    api.lists().then(
      (found) => {
        if (current) setLists(found.filter((each) => !each.inbox));
      },
      () => {
        // Without lists the task goes to the Inbox, which always works.
      },
    );
    return () => {
      current = false;
    };
  }, [api]);

  useEffect(() => {
    if (focusOnMount) input.current?.focus();
  }, [focusOnMount]);

  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const line = text.trim();
    if (line === "" || busy) return;
    setBusy(true);
    setError(undefined);
    api
      .addTask(line, list === "" ? undefined : list)
      // The link goes in after parsing, so words in a page title never read as
      // dates.
      .then((task) =>
        suffix === undefined
          ? task
          : api.update(task.id, { title: `${task.title} ${suffix}` }),
      )
      .then(
        (task) => {
          setBusy(false);
          setText("");
          announceTasksChanged();
          onAdded(task.title);
        },
        (error_: unknown) => {
          setBusy(false);
          setError(failureText(error_));
        },
      );
  };

  const runs = runsOf(text, parsed);
  return (
    <form className="vl-quick-add" onSubmit={submit}>
      <label htmlFor={`${id}-text`}>{t("my-work-add")}</label>
      <input
        ref={input}
        id={`${id}-text`}
        name="text"
        autoComplete="off"
        value={text}
        aria-describedby={`${id}-parse`}
        placeholder={t("my-work-add-hint")}
        onChange={(event) => {
          setText(event.target.value);
        }}
      />
      <div id={`${id}-parse`} className="vl-quick-parse" aria-live="polite">
        {parsed !== undefined && runs.some((run) => run.kind !== undefined) && (
          <>
            <p className="vl-quick-line">
              {runs.map((run, index) =>
                run.kind === undefined ? (
                  // Runs never reorder, so the index is a stable key.

                  <span key={index}>{run.text}</span>
                ) : (
                  <mark key={index} data-kind={run.kind}>
                    {run.text}
                  </mark>
                ),
              )}
            </p>
            <Facts parsed={parsed} />
          </>
        )}
      </div>
      {lists.length > 0 && (
        <label>
          {t("quick-add-to-list")}
          <select
            value={list}
            onChange={(event) => {
              setList(event.target.value);
            }}
          >
            <option value="">{t("my-work-bucket-inbox")}</option>
            {lists.map((each) => (
              <option key={each.doc} value={each.doc}>
                {each.title}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="vl-quick-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("my-work-add-button")}
        </button>
        {children}
      </div>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** What opens the quick-add dialog with. */
export interface QuickAddStart {
  /** Text to start with. */
  readonly text?: string;
  /** The page the task came from: the task links back to it. */
  readonly page?: { readonly id: string; readonly title: string };
}

/** Opens quick-add; `undefined` when the app runs without a Vellum server. */
type OpenQuickAdd = ((start?: QuickAddStart) => void) | undefined;

const QuickAddContext = createContext<OpenQuickAdd>(undefined);

/** Opens quick-add from anywhere: `Q`, the ⌘K menu, or `T` in a page. */
export function useQuickAdd(): OpenQuickAdd {
  return use(QuickAddContext);
}

/** Turns selected text in `page` into a task that links back to it. */
export function usePageTask(
  page: { readonly id: string; readonly title: string } | undefined,
): ((text: string) => void) | undefined {
  const quickAdd = useQuickAdd();
  const id = page?.id;
  const title = page?.title;
  return useMemo(
    () =>
      quickAdd === undefined || id === undefined || title === undefined
        ? undefined
        : (text: string) => {
            quickAdd({ text, page: { id, title } });
          },
    [quickAdd, id, title],
  );
}

/** A Markdown link back to a page, for a task's title. */
function pageLink(page: {
  readonly id: string;
  readonly title: string;
}): string {
  // Brackets and backslashes would end or escape the link text.
  const label = page.title.replaceAll(/[[\]\\]/g, "").trim() || "page";
  return `[${label}](/p/${encodeURIComponent(page.id)})`;
}

/** The quick-add dialog. Escape or Cancel closes it and puts focus back. */
function QuickAddDialog({
  api,
  start,
  onClose,
}: {
  readonly api: WorkApi;
  readonly start: QuickAddStart;
  readonly onClose: () => void;
}) {
  const t = useT();
  const titleId = useId();
  return (
    // Escape closes the modal dialog from wherever focus is inside it.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <div
      className="vl-quick-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <h2 id={titleId}>{t("quick-add-title")}</h2>
      {start.page !== undefined && (
        <p className="vl-muted">
          {t("quick-add-from-page", { page: start.page.title })}
        </p>
      )}
      <QuickAddForm
        api={api}
        initial={start.text ?? ""}
        suffix={start.page === undefined ? undefined : pageLink(start.page)}
        focusOnMount
        onAdded={onClose}
      >
        <button type="button" className="vl-text-button" onClick={onClose}>
          {t("quick-add-cancel")}
        </button>
      </QuickAddForm>
    </div>
  );
}

/** Holds the quick-add dialog for the whole app. */
export function QuickAddProvider({
  children,
}: {
  readonly children: ReactNode;
}) {
  const { work } = useServerApis();
  const [start, setStart] = useState<QuickAddStart>();
  const returnFocus = useRef<Element | null>(null);
  const open = useCallback((next: QuickAddStart = {}) => {
    returnFocus.current = document.activeElement;
    setStart(next);
  }, []);
  const close = useCallback(() => {
    setStart(undefined);
    const back = returnFocus.current;
    if (back instanceof HTMLElement) back.focus();
  }, []);
  const value = useMemo<OpenQuickAdd>(
    () => (work === undefined ? undefined : open),
    [work, open],
  );
  return (
    <QuickAddContext value={value}>
      {children}
      {work !== undefined && start !== undefined && (
        <QuickAddDialog api={work} start={start} onClose={close} />
      )}
    </QuickAddContext>
  );
}
