// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";

import type { Command } from "./keymap";
import { formatBinding, parseBinding, type KeyPlatform } from "./keys";
import { rankCommands } from "./search";

/** Props of the command menu. */
export interface CommandMenuProperties {
  readonly commands: readonly Command[];
  readonly platform: KeyPlatform;
  /** Runs a command; the menu has already closed. */
  readonly onRun: (command: Command) => void;
  readonly onClose: () => void;
}

/** The most results shown at once; typing narrows the rest. */
const LIMIT = 50;

/**
 * The ⌘K menu: type to find any command, arrows to move, Enter to run,
 * Escape to close. It is a modal dialog holding a combobox and its list.
 */
export function CommandMenu({ commands, platform, onRun, onClose }: CommandMenuProperties) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const listId = useId();
  const optionId = useId();
  const results = useMemo(() => rankCommands(commands, query).slice(0, LIMIT), [commands, query]);
  const current = Math.min(active, Math.max(results.length - 1, 0));

  useEffect(() => {
    input.current?.focus();
  }, []);

  const run = (command: Command | undefined) => {
    if (command !== undefined) {
      onClose();
      onRun(command);
    }
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
    <div className="command-menu" role="dialog" aria-modal="true" aria-label="Command menu">
      <input
        ref={input}
        type="text"
        role="combobox"
        aria-label="Search commands"
        aria-expanded="true"
        aria-controls={listId}
        aria-activedescendant={results.length > 0 ? `${optionId}-${String(current)}` : undefined}
        aria-autocomplete="list"
        placeholder="Type a command"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          setActive(0);
        }}
        onKeyDown={onKeyDown}
      />
      <div id={listId} role="listbox" aria-label="Commands">
        {results.map((command, index) => (
          <div
            key={command.id}
            id={`${optionId}-${String(index)}`}
            role="option"
            tabIndex={-1}
            aria-selected={index === current}
            onMouseDown={(event) => {
              // Keep focus in the search box; the click runs the command.
              event.preventDefault();
              run(command);
            }}
            onMouseEnter={() => {
              setActive(index);
            }}
          >
            <span>{command.title}</span>
            {command.keys[0] === undefined ? undefined : (
              <kbd>{formatBinding(parseBinding(command.keys[0]), platform)}</kbd>
            )}
          </div>
        ))}
      </div>
      {results.length === 0 ? <p role="status">No command matches “{query}”.</p> : undefined}
    </div>
  );
}
