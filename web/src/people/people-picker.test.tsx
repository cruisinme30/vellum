// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { I18nProvider } from "../i18n/i18n";

import {
  combineDirectories,
  createHttpDirectory,
  createHttpGroupDirectory,
  createListDirectory,
  isPrincipal,
  type DirectoryApi,
} from "./directory";
import { PeoplePicker } from "./people-picker";

const PEOPLE = createListDirectory([
  {
    kind: "user",
    principal: "user:01PRIYA",
    name: "Priya Raman",
    detail: "priya@northstar.dev",
  },
  {
    kind: "user",
    principal: "user:01PRIA",
    name: "Pria Stone",
    detail: "pria@northstar.dev",
  },
]);
const GROUPS = createListDirectory([
  {
    kind: "group",
    principal: "group:01SRE",
    name: "SRE",
    detail: "12 members",
  },
]);

function Harness({
  directory,
  initial = [],
  onChange,
}: {
  readonly directory?: DirectoryApi;
  readonly initial?: string[];
  readonly onChange?: (value: string[]) => void;
}) {
  const [value, setValue] = useState(initial);
  return (
    <PeoplePicker
      label="Reviewers"
      value={value}
      directory={directory}
      onChange={(next) => {
        setValue(next);
        onChange?.(next);
      }}
    />
  );
}

function renderPicker(properties: Parameters<typeof Harness>[0]) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <Harness {...properties} />
    </I18nProvider>,
  );
  return user;
}

describe("people picker", () => {
  it("searches as you type and picks with the keyboard", async () => {
    const onChange = vi.fn();
    const user = renderPicker({ directory: PEOPLE, onChange });
    const box = screen.getByRole("combobox", { name: "Reviewers" });
    await user.type(box, "pri");
    expect(await screen.findAllByRole("option")).toHaveLength(2);
    expect(box.getAttribute("aria-expanded")).toBe("true");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(["user:01PRIA"]);
    const chips = screen.getByRole("list", { name: "Chosen for Reviewers" });
    expect(within(chips).getByText("Pria Stone")).toBeDefined();
    // A chosen person is not offered again.
    await user.type(box, "pri");
    expect(await screen.findAllByRole("option")).toHaveLength(1);
    await user.keyboard("{Escape}");
    expect(box.getAttribute("aria-expanded")).toBe("false");
  });

  it("removes with the chip button or Backspace", async () => {
    const onChange = vi.fn();
    const user = renderPicker({
      directory: PEOPLE,
      initial: ["user:A", "group:B"],
      onChange,
    });
    await user.click(screen.getByRole("button", { name: "Remove group:B" }));
    expect(onChange).toHaveBeenLastCalledWith(["user:A"]);
    await user.keyboard("{Backspace}");
    expect(onChange).toHaveBeenLastCalledWith([]);
  });

  it("takes a full ID typed in, and says when nobody matches", async () => {
    const onChange = vi.fn();
    const user = renderPicker({ directory: PEOPLE, onChange });
    await user.type(screen.getByRole("combobox"), "zz");
    expect(await screen.findByText("No one found.")).toBeDefined();
    await user.clear(screen.getByRole("combobox"));
    await user.type(screen.getByRole("combobox"), "group:01X{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(["group:01X"]);
  });

  it("works without a directory and reports a failed search", async () => {
    const failing: DirectoryApi = {
      search: () => Promise.reject(new Error("down")),
    };
    const user = renderPicker({ directory: failing });
    await user.type(screen.getByRole("combobox"), "pri");
    expect(await screen.findByRole("alert")).toBeDefined();
  });
});

describe("directory", () => {
  it("asks the server", async () => {
    const get = vi.fn(() => Promise.resolve({ entries: [] }));
    const api = createHttpDirectory({
      get,
      send: vi.fn(),
      forgetSession: vi.fn(),
    } as unknown as Http);
    await api.search("pri");
    expect(get).toHaveBeenCalledWith("/api/v1/directory", { q: "pri" });
  });

  it("asks the server for groups", async () => {
    const entry = {
      kind: "group",
      principal: "group:01SRE",
      name: "SRE",
      detail: "On call",
    };
    const get = vi.fn(() => Promise.resolve({ entries: [entry] }));
    const api = createHttpGroupDirectory({
      get,
      send: vi.fn(),
      forgetSession: vi.fn(),
    } as unknown as Http);
    expect(await api.search("sr")).toEqual([entry]);
    expect(get).toHaveBeenCalledWith("/api/v1/directory/groups", {
      q: "sr",
      limit: 8,
    });
  });

  it("merges people and groups, skipping a failed source", async () => {
    const failing: DirectoryApi = {
      search: () => Promise.reject(new Error("down")),
    };
    const both = combineDirectories(PEOPLE, GROUPS, failing, PEOPLE);
    const found = await both.search("r");
    expect(found).toEqual([]);
    const all = await both.search("sre");
    expect(all.map((entry) => entry.principal)).toEqual(["group:01SRE"]);
    const people = await both.search("pri");
    expect(people.map((entry) => entry.principal)).toEqual([
      "user:01PRIYA",
      "user:01PRIA",
    ]);
  });

  it("recognises typed principals", () => {
    expect(isPrincipal("user:01ABC")).toBe(true);
    expect(isPrincipal(" group:x-1 ")).toBe(true);
    expect(isPrincipal("priya")).toBe(false);
    expect(isPrincipal("user:")).toBe(false);
  });
});
