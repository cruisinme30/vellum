// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/* eslint-disable @typescript-eslint/unbound-method -- the API's methods are vi.fn mocks */

import { screen, waitFor, within } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../../api/http";
import { fakeApi, issue, renderAt, TEAM, TEAM_FILE, where } from "../../test/work-fixtures";
import { createHttpWorkApi } from "../api";

import { freshKey, moveWithinCategory, pruneGit, slug } from "./team-file";
import { TeamSettingsPage } from "./team-settings-page";

const route = <Route path="issues/settings" element={<TeamSettingsPage />} />;

function saved(api: ReturnType<typeof fakeApi>) {
  const file = vi.mocked(api.saveTeamSettings).mock.calls.at(-1)?.[1];
  if (file === undefined) throw new Error("not saved");
  return file;
}

/** A promise the test settles when it is ready. */
function deferred<T>() {
  const settle: { resolve?: (value: T) => void } = {};
  const promise = new Promise<T>((resolve) => {
    settle.resolve = resolve;
  });
  return {
    promise,
    resolve: (value: T) => {
      settle.resolve?.(value);
    },
  };
}

describe("team settings file", () => {
  it("makes keys, reorders within a category and drops dangling Git moments", () => {
    expect(slug("In Review!")).toBe("in-review");
    expect(freshKey("Todo", ["todo", "todo-2"])).toBe("todo-3");
    const statuses = TEAM_FILE.workflow.statuses;
    const moved = moveWithinCategory(statuses, "next", -1).map((status) => status.key);
    expect(moved.slice(1, 3)).toEqual(["next", "todo"]);
    // Nothing above "triage" in its own category: unchanged.
    expect(moveWithinCategory(statuses, "triage", -1)).toEqual(statuses);
    const pruned = pruneGit({ ...TEAM_FILE, git: { pr_merged: "done", pr_opened: "gone" } });
    expect(pruned.git).toEqual({ pr_merged: "done" });
  });
});

describe("team settings API", () => {
  it("reads the file and saves it back as YAML text", async () => {
    const get = vi.fn(() => Promise.resolve({ settings: TEAM_FILE, yaml: "" }));
    const send = vi.fn(() => Promise.resolve({}));
    const api = createHttpWorkApi({ get, send } as unknown as Http);
    expect(await api.teamSettings("ENG")).toBe(TEAM_FILE);
    expect(get).toHaveBeenCalledWith("/api/v1/work/teams/ENG/settings");
    await api.saveTeamSettings("ENG", TEAM_FILE);
    expect(send).toHaveBeenCalledWith("PUT", "/api/v1/work/teams/ENG", {
      yaml: JSON.stringify(TEAM_FILE),
    });
    await api.createTeam({ key: "OPS", name: "Ops" });
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/work/teams", { key: "OPS", name: "Ops" });
    await api.setTeamArchived("OPS", true);
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/work/teams/OPS/archive", {});
    await api.setTeamArchived("OPS", false);
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/work/teams/OPS/restore", {});
  });

  it("lists active teams unless archived ones are asked for", async () => {
    const get = vi.fn(() => Promise.resolve({ teams: [] }));
    const api = createHttpWorkApi({ get, send: vi.fn() } as unknown as Http);
    await api.teams();
    await api.teams({ includeArchived: true });
    expect(get.mock.calls).toEqual([
      ["/api/v1/work/teams"],
      ["/api/v1/work/teams", { include_archived: "true" }],
    ]);
  });
});

describe("team settings screen", () => {
  it("edits statuses, cycles, SLAs, estimates and Git, then saves the whole file", async () => {
    const api = fakeApi();
    const user = renderAt("/issues/settings", api, route);
    const name = await screen.findByRole("textbox", { name: "Name of the todo status" });
    await user.clear(name);
    await user.type(name, "Up next");
    await user.type(screen.getByRole("textbox", { name: "New Started status" }), "QA{Enter}");
    await user.click(screen.getByRole("button", { name: "Move Next up" }));
    await user.click(screen.getByRole("checkbox", { name: "Run cycles" }));
    const weeks = screen.getByRole("spinbutton", { name: "Length in weeks" });
    await user.clear(weeks);
    await user.type(weeks, "1");
    await user.selectOptions(screen.getByRole("combobox", { name: "Starts on" }), "tuesday");
    await user.click(screen.getByRole("button", { name: "Add SLA rule" }));
    const hours = screen.getAllByRole("spinbutton", { name: "Within hours" }).at(-1);
    if (hours === undefined) throw new Error("no SLA rule");
    await user.clear(hours);
    await user.type(hours, "72");
    await user.selectOptions(screen.getByRole("combobox", { name: "Scale" }), "t-shirt");
    await user.selectOptions(screen.getByRole("combobox", { name: "Pull request opened" }), "qa");
    await user.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() => {
      expect(api.saveTeamSettings).toHaveBeenCalled();
    });
    const file = saved(api);
    const keys = file.workflow.statuses.map((status) => status.key);
    expect(keys).toEqual([
      "triage",
      "next",
      "todo",
      "doing",
      "qa",
      "done",
      "canceled",
      "duplicate",
    ]);
    expect(file.workflow.statuses.find((s) => s.key === "todo")?.name).toBe("Up next");
    expect(file.cycles).toMatchObject({ duration_weeks: 1, start_day: "tuesday" });
    expect(file.sla?.rules?.at(-1)).toMatchObject({ within_hours: 72 });
    expect(file.estimates?.scale).toBe("t-shirt");
    expect(file.git).toEqual({ pr_opened: "qa" });
    // What the screen does not edit rides along.
    expect(file["labels"]).toEqual([{ name: "gateway" }]);
    expect(file["last_number"]).toBe(9);
    expect(await screen.findByText("Settings saved.")).toBeDefined();
  });

  it("moves a removed status's issues before saving", async () => {
    const api = fakeApi({
      queryIssues: vi
        .fn()
        .mockResolvedValueOnce([{ issues: [issue("ENG-3", "a", { state: "next" })] }])
        .mockResolvedValueOnce([{ issues: [issue("ENG-3", "a", { state: "next" })] }])
        .mockResolvedValue([{ issues: [] }]),
    });
    const user = renderAt("/issues/settings", api, route);
    await user.click(await screen.findByRole("button", { name: "Remove Next" }));
    const ask = await screen.findByRole("group", { name: "Remove Next" });
    expect(ask.textContent).toContain("One issue is in Next");
    await user.selectOptions(within(ask).getByRole("combobox"), "doing");
    await user.click(within(ask).getByRole("button", { name: "Remove and move issues" }));
    expect(screen.queryByRole("textbox", { name: "Name of the next status" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => {
      expect(api.saveTeamSettings).toHaveBeenCalled();
    });
    expect(api.setState).toHaveBeenCalledWith("ENG-3", "doing");
    expect(vi.mocked(api.setState).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(api.saveTeamSettings).mock.invocationCallOrder[0] ?? 0,
    );
    expect(saved(api).workflow.statuses.some((s) => s.key === "next")).toBe(false);
  });

  it("shows a refused save and creates a team", async () => {
    const api = fakeApi({
      saveTeamSettings: vi.fn(() =>
        Promise.reject(new Error("default status must be backlog or unstarted")),
      ),
    });
    const user = renderAt("/issues/settings", api, route);
    await user.click(await screen.findByRole("checkbox", { name: "Use a triage inbox" }));
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("default status");

    await user.click(screen.getByRole("button", { name: "New team" }));
    const form = screen.getByRole("form", { name: "New team" });
    await user.type(within(form).getByRole("textbox", { name: "Name" }), "Platform");
    await user.type(within(form).getByRole("textbox", { name: "Key prefix" }), "eng2");
    await user.type(within(form).getByRole("textbox", { name: "Members" }), "@dev, priya");
    await user.click(within(form).getByRole("button", { name: "Create team" }));
    expect(api.createTeam).toHaveBeenCalledWith({
      key: "ENG2",
      name: "Platform",
      members: ["dev", "priya"],
    });
    await waitFor(() => {
      expect(where()).toBe("/issues/settings?team=ENG2");
    });
  });

  it("archives a team after asking, then shows it archived with Restore", async () => {
    let archived = false;
    const api = fakeApi({
      teams: vi.fn(() =>
        Promise.resolve([
          { ...TEAM, archived },
          { ...TEAM, id: "t2", key: "OPS", name: "Operations" },
        ]),
      ),
      setTeamArchived: vi.fn((key: string, value: boolean) => {
        archived = value;
        return Promise.resolve({ ...TEAM, key, archived: value });
      }),
    });
    const user = renderAt("/issues/settings", api, route);
    await user.click(await screen.findByRole("button", { name: "Archive Engineering" }));
    const ask = screen.getByRole("group", { name: "Archive Engineering?" });
    expect(api.setTeamArchived).not.toHaveBeenCalled();
    await user.click(within(ask).getByRole("button", { name: "Archive team" }));
    expect(api.setTeamArchived).toHaveBeenCalledWith("ENG", true);
    // The screen stays on the team, now archived, with a way back.
    expect(await screen.findByText(/Engineering is archived/)).toBeDefined();
    expect(where()).toBe("/issues/settings?team=ENG");
    expect(screen.getByRole("option", { name: "Engineering (archived)" })).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Restore team" }));
    expect(api.setTeamArchived).toHaveBeenLastCalledWith("ENG", false);
    expect(await screen.findByRole("button", { name: "Archive Engineering" })).toBeDefined();
  });

  it("shows the archive section only once the settings have loaded", async () => {
    // Regression: it rendered first and jumped down the page (layout shift).
    const settings = deferred<typeof TEAM_FILE>();
    const api = fakeApi({ teamSettings: vi.fn(() => settings.promise) });
    renderAt("/issues/settings", api, route);
    await waitFor(() => {
      expect(api.teamSettings).toHaveBeenCalled();
    });
    expect(screen.queryByRole("button", { name: "Archive Engineering" })).toBeNull();
    settings.resolve(TEAM_FILE);
    expect(await screen.findByRole("button", { name: "Archive Engineering" })).toBeDefined();
  });

  it("hides archived teams from the picker until asked, and shows a refused archive", async () => {
    const api = fakeApi({
      teams: vi.fn(() =>
        Promise.resolve([TEAM, { ...TEAM, id: "t2", key: "OLD", name: "Legacy", archived: true }]),
      ),
      setTeamArchived: vi.fn(() => Promise.reject(new Error("a recent sign-in is needed"))),
    });
    const user = renderAt("/issues/settings", api, route);
    const picker = await screen.findByRole("combobox", { name: "Team" });
    expect(within(picker).queryByRole("option", { name: "Legacy (archived)" })).toBeNull();
    await user.click(screen.getByRole("checkbox", { name: "Show archived teams" }));
    expect(within(picker).getByRole("option", { name: "Legacy (archived)" })).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Archive Engineering" }));
    await user.click(screen.getByRole("button", { name: "Archive team" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("recent sign-in");
  });
});
