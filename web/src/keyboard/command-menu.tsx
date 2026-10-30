// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

import { highlight } from "../data/search";
import { telemetry } from "../platform/telemetry";
import { Marked } from "../views/page-list";

import type { Command } from "./keymap";
import { formatBinding, parseBinding, type KeyPlatform } from "./keys";
import { rankCommands } from "./search";

/** A page the menu can open. */
export interface FoundPage {
  readonly id: string;
  readonly title: string;
  readonly icon?: string;
}

/**
 * Page search in the menu (R-SR-1). `match` must answer synchronously from
 * memory: it runs on every keystroke and has 20 ms.
 */
export interface PageFinder {
  readonly match: (query: string) => readonly FoundPage[];
  readonly open: (id: string) => void;
  /** Opens full-text search for the query. */
  readonly searchText: (query: string) => void;
}

/** One row of the menu. */
type Option =
  | { readonly kind: "page"; readonly key: string; readonly page: FoundPage }
  | { readonly kind: "text"; readonly key: string }
  | {
      readonly kind: "command";
      readonly key: string;
      readonly command: Command;
    };

/** Props of the command menu. */
export interface CommandMenuProperties {
  readonly commands: readonly Command[];
  /** Pages to offer as the person types; commands only when absent. */
  readonly pages?: PageFinder | undefined;
  readonly platform: KeyPlatform;
  /** Runs a command; the menu has already closed. */
  readonly onRun: (command: Command) => void;
  readonly onClose: () => void;
}

/** The most results shown at once; typing narrows the rest. */
const LIMIT = 50;

/** Pages matching the query, timed against the 20 ms budget. */
function findPages(
  pages: PageFinder | undefined,
  query: string,
): readonly FoundPage[] {
  if (pages === undefined || query.trim() === "") return [];
  const started = performance.now();
  const found = pages.match(query);
  telemetry().histogram("ui.search.titles", performance.now() - started, {
    hits: String(found.length),
  });
  return found;
}

/**
 * The ⌘K menu: type to find a page or any command, arrows to move, Enter to
 * open or run, Escape to close. It is a modal dialog holding a combobox and
 * its list. Matching page titles come first, then "search every page for
 * …", then commands.
 */
export function CommandMenu({
  commands,
  pages,
  platform,
  onRun,
  onClose,
}: CommandMenuProperties) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const listId = useId();
  const optionId = useId();
  const results = useMemo<Option[]>(() => {
    const found: Option[] = findPages(pages, query).map((page) => ({
      kind: "page",
      key: `page:${page.id}`,
      page,
    }));
    if (pages !== undefined && query.trim() !== "")
      found.push({ kind: "text", key: "text" });
    for (const command of rankCommands(commands, query)) {
      found.push({ kind: "command", key: `command:${command.id}`, command });
    }
    return found.slice(0, LIMIT);
  }, [commands, pages, query]);
  const current = Math.min(active, Math.max(results.length - 1, 0));

  useEffect(() => {
    input.current?.focus();
  }, []);

  const run = (option: Option | undefined) => {
    if (option === undefined) return;
    onClose();
    if (option.kind === "command") onRun(option.command);
    else if (option.kind === "page") pages?.open(option.page.id);
    else pages?.searchText(query.trim());
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        setActive((current + 1) % Math.max(results.length, 1));
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        setActive((current - 1 + results.length) % Math.max(results.length, 1));
        break;
      }
      case "Enter": {
        event.preventDefault();
        run(results[current]);
        break;
      }
      case "Escape": {
        event.preventDefault();
        onClose();
        break;
      }
      default: {
        break;
      }
    }
  };

  return (
    <div
      className="command-menu"
      role="dialog"
      aria-modal="true"
      aria-label="Command menu"
    >
      <input
        ref={input}
        type="text"
        role="combobox"
        aria-label={
          pages === undefined ? "Search commands" : "Search pages and commands"
        }
        aria-expanded="true"
        aria-controls={listId}
        aria-activedescendant={
          results.length > 0 ? `${optionId}-${String(current)}` : undefined
        }
        aria-autocomplete="list"
        placeholder={
          pages === undefined
            ? "Type a command"
            : "Type a page title or a command"
        }
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          setActive(0);
        }}
        onKeyDown={onKeyDown}
      />
      <div id={listId} role="listbox" aria-label="Results">
        {results.map((option, index) => (
          <div
            key={option.key}
            id={`${optionId}-${String(index)}`}
            role="option"
            tabIndex={-1}
            aria-selected={index === current}
            onMouseDown={(event) => {
              // Keep focus in the search box; the click runs the command.
              event.preventDefault();
              run(option);
            }}
            onMouseEnter={() => {
              setActive(index);
            }}
          >
            <OptionRow option={option} query={query} platform={platform} />
          </div>
        ))}
      </div>
      {results.length === 0 ? (
        <p role="status">Nothing matches “{query}”.</p>
      ) : undefined}
    </div>
  );
}

/**
 * What one row shows: a page with its matched words marked, the full-text
 * search, or a command.
 */
function OptionRow({
  option,
  query,
  platform,
}: {
  readonly option: Option;
  readonly query: string;
  readonly platform: KeyPlatform;
}) {
  switch (option.kind) {
    case "page": {
      return (
        <>
          <span>
            <span aria-hidden="true">{option.page.icon ?? "📄"} </span>
            <Marked
              fragments={highlight(option.page.title || "Untitled", query)}
            />
          </span>
          <small>Page</small>
        </>
      );
    }
    case "text": {
      return <span>Search every page for “{query.trim()}”</span>;
    }
    case "command": {
      const [binding] = option.command.keys;
      return (
        <>
          <span>{option.command.title}</span>
          {binding === undefined ? undefined : (
            <kbd>{formatBinding(parseBinding(binding), platform)}</kbd>
          )}
        </>
      );
    }
  }
}
