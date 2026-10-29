// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";

import { useT } from "../i18n/i18n";

import { isPrincipal, type DirectoryApi, type DirectoryEntry } from "./directory";

import "./people.css";

/** How long typing pauses before the directory is asked, in milliseconds. */
const DEBOUNCE_MS = 200;

interface PeoplePickerProperties {
  /** The field's visible label. */
  readonly label: string;
  /** Chosen principals (`user:<id>`, `group:<id>`), in the order chosen. */
  readonly value: readonly string[];
  readonly onChange: (value: string[]) => void;
  /** Where to search; without one, only full `user:` or `group:` IDs can be typed. */
  readonly directory?: DirectoryApi | undefined;
  /** Help text under the field. */
  readonly hint?: string | undefined;
  readonly disabled?: boolean | undefined;
}

/** What the directory answered for one query. */
interface Answer {
  readonly query: string;
  readonly found: readonly DirectoryEntry[];
  readonly failed: boolean;
}

/** The directory's answer for `query`, once typing has paused. */
function useSearch(directory: DirectoryApi | undefined, query: string): Answer | undefined {
  const [answer, setAnswer] = useState<Answer>();
  useEffect(() => {
    if (directory === undefined || query.length < 2) return;
    let current = true;
    const timer = setTimeout(() => {
      directory.search(query).then(
        (found) => {
          if (current) setAnswer({ query, found, failed: false });
        },
        () => {
          if (current) setAnswer({ query, found: [], failed: true });
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [directory, query]);
  return answer?.query === query ? answer : undefined;
}

const shownName = (entry: DirectoryEntry, t: ReturnType<typeof useT>) =>
  entry.kind === "group" ? t("people-group", { name: entry.name }) : entry.name;

function Chips({
  label,
  value,
  names,
  disabled,
  onRemove,
}: {
  readonly label: string;
  readonly value: readonly string[];
  readonly names: ReadonlyMap<string, DirectoryEntry>;
  readonly disabled: boolean | undefined;
  readonly onRemove: (principal: string) => void;
}) {
  const t = useT();
  if (value.length === 0) return;
  return (
    <ul className="vl-people-chips" aria-label={t("people-chosen", { label })}>
      {value.map((principal) => {
        const entry = names.get(principal);
        const name = entry === undefined ? principal : shownName(entry, t);
        return (
          <li key={principal} className="vl-people-chip">
            <span title={entry?.detail ?? principal}>{name}</span>
            <button
              type="button"
              className="vl-people-remove"
              aria-label={t("people-remove", { name })}
              disabled={disabled}
              onClick={() => {
                onRemove(principal);
              }}
            >
              ×
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function Options({
  id,
  matches,
  active,
  shown,
  onPick,
}: {
  readonly id: string;
  readonly matches: readonly DirectoryEntry[];
  readonly active: number;
  readonly shown: boolean;
  readonly onPick: (entry: DirectoryEntry) => void;
}) {
  const t = useT();
  return (
    <div id={`${id}-list`} role="listbox" className="vl-people-list" hidden={!shown}>
      {matches.map((entry, index) => (
        <div
          key={entry.principal}
          id={`${id}-option-${String(index)}`}
          role="option"
          tabIndex={-1}
          aria-selected={index === active}
          onMouseDown={(event) => {
            // Keep focus in the field; the press picks the option.
            event.preventDefault();
            onPick(entry);
          }}
        >
          <span>{shownName(entry, t)}</span>
          <small className="vl-muted">{entry.detail}</small>
        </div>
      ))}
    </div>
  );
}

function Messages({
  id,
  count,
  none,
  failed,
  hint,
}: {
  readonly id: string;
  /** How many matches the open list shows, if it is open. */
  readonly count: number | undefined;
  readonly none: boolean;
  readonly failed: boolean;
  readonly hint: string | undefined;
}) {
  const t = useT();
  return (
    <>
      <p className="vl-visually-hidden" role="status">
        {count === undefined ? "" : t("people-found", { count })}
      </p>
      {none && <p className="vl-muted">{t("people-none")}</p>}
      {failed && (
        <p className="vl-field-error" role="alert">
          {t("people-failed")}
        </p>
      )}
      {hint !== undefined && (
        <p id={`${id}-hint`} className="vl-muted">
          {hint}
        </p>
      )}
    </>
  );
}

/**
 * Picks people and groups: a WAI-ARIA combobox that searches the directory
 * as you type, with the chosen ones as removable chips. Arrow keys move
 * through the matches, Enter adds one, Escape closes the list, and
 * Backspace in an empty field removes the last chip. A full `user:<id>` or
 * `group:<id>` can always be typed, for principals the directory does not
 * list.
 *
 * The one picker for book section access, publishing, the share dialog
 * and the admin screens; give it a {@link DirectoryApi} (groups join
 * through `combineDirectories`).
 */
export function PeoplePicker({
  label,
  value,
  onChange,
  directory,
  hint,
  disabled,
}: PeoplePickerProperties) {
  const t = useT();
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [open, setOpen] = useState(false);
  // Names learned from searches, so chips read as people, not IDs.
  const [names, setNames] = useState<ReadonlyMap<string, DirectoryEntry>>(new Map());
  const q = query.trim();
  const answer = useSearch(directory, q);
  const matches = (answer?.found ?? []).filter((entry) => !value.includes(entry.principal));
  const shown = open && matches.length > 0;

  const add = (principal: string, entry?: DirectoryEntry) => {
    if (!value.includes(principal)) onChange([...value, principal]);
    if (entry !== undefined) setNames((known) => new Map(known).set(principal, entry));
    setQuery("");
    setOpen(false);
    input.current?.focus();
  };
  const remove = (principal: string) => {
    onChange(value.filter((p) => p !== principal));
    input.current?.focus();
  };
  const step = (by: number) => {
    setOpen(true);
    if (matches.length > 0) setActive((a) => (a + by + matches.length) % matches.length);
  };

  const keys: Readonly<Record<string, () => boolean>> = {
    ArrowDown: () => {
      step(shown ? 1 : 0);
      return true;
    },
    ArrowUp: () => {
      step(-1);
      return shown;
    },
    Enter: () => {
      const chosen = shown ? matches[active] : undefined;
      if (chosen !== undefined) add(chosen.principal, chosen);
      else if (isPrincipal(q)) add(q);
      else return false;
      return true;
    },
    Escape: () => {
      setOpen(false);
      return open;
    },
    Backspace: () => {
      const last = value.at(-1);
      if (query === "" && last !== undefined) remove(last);
      return false;
    },
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (keys[event.key]?.() === true) event.preventDefault();
  };

  return (
    <div className="vl-people">
      <label htmlFor={`${id}-input`}>{label}</label>
      <Chips label={label} value={value} names={names} disabled={disabled} onRemove={remove} />
      <input
        ref={input}
        id={`${id}-input`}
        role="combobox"
        aria-expanded={shown}
        aria-controls={`${id}-list`}
        aria-autocomplete="list"
        aria-activedescendant={shown ? `${id}-option-${String(active)}` : undefined}
        aria-describedby={hint === undefined ? undefined : `${id}-hint`}
        autoComplete="off"
        disabled={disabled}
        value={query}
        placeholder={directory === undefined ? "user:…" : t("people-search")}
        onChange={(event) => {
          setQuery(event.currentTarget.value);
          setActive(0);
          setOpen(true);
        }}
        onFocus={() => {
          setOpen(true);
        }}
        onBlur={() => {
          setOpen(false);
        }}
        onKeyDown={onKeyDown}
      />
      <Options
        id={id}
        matches={matches}
        active={active}
        shown={shown}
        onPick={(entry) => {
          add(entry.principal, entry);
        }}
      />
      <Messages
        id={id}
        count={shown ? matches.length : undefined}
        none={open && answer?.failed === false && matches.length === 0}
        failed={answer?.failed === true}
        hint={hint}
      />
    </div>
  );
}
