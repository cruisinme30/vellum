// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/* eslint-disable @typescript-eslint/unbound-method -- the APIs' methods are vi.fn mocks */

import { render, screen, waitFor, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../../api/http";
import { createNorthstarStore } from "../../data/mock/northstar";
import { ServerApisProvider } from "../../data/server-apis";
import { WorkspaceProvider } from "../../data/workspace-context";
import type { CollabConnector } from "../../editor/collab/provider";
import { I18nProvider } from "../../i18n/i18n";
import { KeyboardRoot } from "../../keyboard/keyboard-root";
import { fakeApi, TEAM_FILE } from "../../test/work-fixtures";
import type { WorkApi } from "../api";
import { IssuePage } from "../issue-page";
import { createHttpPlanningApi, type PlanningApi } from "../planning/api";
import { fakeApi as fakePlanning, GATEWAY, issue } from "../planning/fixtures";
import type { TeamFile } from "../settings/team-file";

import {
  copyName,
  formOf,
  isRepeat,
  moved,
  problemOf,
  repeatChoices,
  templateOf,
  type Template,
} from "./template-file";
import { TemplatesPage } from "./templates-page";

const BUG: Template = {
  name: "Bug report",
  title: "Bug:",
  state: "triage",
  priority: 2,
  labels: ["gateway"],
  body: "Steps to reproduce:",
};

const HANDOFF: Template = {
  name: "On-call handoff",
  assignee: "mara",
  project: "p1",
  milestone: "m1",
  repeat: "1w",
};

const collab: CollabConnector = {
  open: () => {
    throw new Error("no editor in these tests");
  },
};

const FILE: TeamFile = { ...TEAM_FILE, templates: [BUG, HANDOFF] };

function renderScreen(path: string, work: WorkApi, planning?: PlanningApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <WorkspaceProvider store={createNorthstarStore()} collab={collab}>
        <ServerApisProvider apis={{ work, planning }}>
          <KeyboardRoot>
            <MemoryRouter initialEntries={[path]}>
              <Routes>
                <Route path="issues/templates" element={<TemplatesPage />} />
                <Route path="issues/:issueKey" element={<IssuePage />} />
              </Routes>
            </MemoryRouter>
          </KeyboardRoot>
        </ServerApisProvider>
      </WorkspaceProvider>
    </I18nProvider>,
  );
  return user;
}

function savedTemplates(api: WorkApi): readonly Template[] | undefined {
  return vi.mocked(api.saveTeamSettings).mock.calls.at(-1)?.[1].templates;
}

describe("template file", () => {
  it("checks repeats the way the server does", () => {
    expect(isRepeat("2w")).toBe(true);
    expect(isRepeat("366d")).toBe(true);
    expect(isRepeat("0w")).toBe(false);
    expect(isRepeat("367d")).toBe(false);
    expect(isRepeat("2y")).toBe(false);
    expect(repeatChoices("3d")).toContain("3d");
    expect(repeatChoices("1w")).toEqual(["1d", "1w", "2w", "1m", "3m"]);
  });

  it("round-trips a template through its form and keeps the milestone", () => {
    expect(templateOf(formOf(HANDOFF), HANDOFF)).toEqual(HANDOFF);
    expect(templateOf(formOf(BUG), BUG)).toEqual(BUG);
    // A new project drops the old project's milestone.
    const moved_ = templateOf({ ...formOf(HANDOFF), project: "p2" }, HANDOFF);
    expect(moved_.milestone).toBeUndefined();
    expect(templateOf({ ...formOf(), name: " Chore ", estimate: "m" })).toEqual({
      name: "Chore",
      estimate: "M",
    });
  });

  it("names what is wrong with a form", () => {
    const ok = formOf(BUG);
    expect(problemOf(ok, [])).toBeUndefined();
    expect(problemOf({ ...ok, name: " " }, [])).toBe("tpl-error-name");
    expect(problemOf(ok, ["bug REPORT"])).toBe("tpl-error-name-taken");
    expect(problemOf({ ...ok, repeat: "9y" }, [])).toBe("tpl-error-repeat");
    expect(problemOf({ ...ok, estimate: "lots" }, [])).toBe("tpl-error-estimate");
    expect(problemOf({ ...ok, assignee: "@-x" }, [])).toBe("tpl-error-assignee");
    expect(problemOf({ ...ok, name: "x".repeat(81) }, [])).toBe("tpl-error-name-long");
  });

  it("copies and moves", () => {
    expect(copyName("Bug", ["Bug", "Bug copy"])).toBe("Bug copy 2");
    expect(moved(["a", "b", "c"], 2, -1)).toEqual(["a", "c", "b"]);
    expect(moved(["a", "b"], 0, -1)).toEqual(["a", "b"]);
  });
});

describe("planning API for repeats", () => {
  it("asks for repeating issues and sets a repeat with its due date", async () => {
    const get = vi.fn(() => Promise.resolve({ groups: [{ issues: [issue("ENG-4", "Rotate")] }] }));
    const send = vi.fn(() => Promise.resolve(issue("ENG-4", "Rotate")));
    const api = createHttpPlanningApi({ get, send } as unknown as Http);
    const found = await api.recurring("ENG");
    expect(found.map((each) => each.key)).toEqual(["ENG-4"]);
    expect(get).toHaveBeenCalledWith("/api/v1/work/issues", {
      team: "ENG",
      filter: JSON.stringify({ field: "repeat", op: "is_not_empty" }),
      group_by: "none",
      order_by: "due",
      show_closed: "none",
    });
    await api.setRepeat("ENG-4", "2w", "2026-11-02");
    expect(send).toHaveBeenCalledWith("PATCH", "/api/v1/work/issues/ENG-4", {
      due: "2026-11-02",
      repeat: "2w",
    });
    await api.setRepeat("ENG-4", "");
    expect(send).toHaveBeenLastCalledWith("PATCH", "/api/v1/work/issues/ENG-4", { repeat: "" });
  });
});

describe("templates screen", () => {
  it("lists the team's templates with what they fill in", async () => {
    const work = fakeApi({ teamSettings: vi.fn(() => Promise.resolve(FILE)) });
    renderScreen("/issues/templates", work, fakePlanning());
    const heading = await screen.findByRole("heading", { name: "Bug report" });
    const bug = heading.closest("li");
    if (bug === null) throw new Error("no row");
    expect(within(bug).getByText("Starts in Triage")).toBeDefined();
    expect(within(bug).getByText("Priority: High")).toBeDefined();
    expect(within(bug).getByText("Labels: gateway")).toBeDefined();
    expect(screen.getByText("Assigned to @mara")).toBeDefined();
    expect(screen.getByText("Repeats every 1w")).toBeDefined();
  });

  it("creates a template and saves the whole team file", async () => {
    const work = fakeApi({ teamSettings: vi.fn(() => Promise.resolve(FILE)) });
    const planning = fakePlanning({ projects: vi.fn(() => Promise.resolve([GATEWAY])) });
    const user = renderScreen("/issues/templates", work, planning);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const form = screen.getByRole("form", { name: "New template" });
    await user.type(within(form).getByLabelText("Template name"), "Release checklist");
    await user.selectOptions(within(form).getByLabelText("Starting status"), "next");
    await user.selectOptions(within(form).getByLabelText("Priority"), "1");
    await user.selectOptions(within(form).getByLabelText("Project"), "p1");
    await user.selectOptions(within(form).getByLabelText("Repeat"), "2w");
    await user.type(within(form).getByLabelText("Labels, separated by commas"), "release, web");
    await user.type(within(form).getByLabelText("Description"), "Tag the build");
    await user.click(within(form).getByRole("button", { name: "Save template" }));
    await waitFor(() => {
      expect(work.saveTeamSettings).toHaveBeenCalled();
    });
    const file = vi.mocked(work.saveTeamSettings).mock.calls.at(-1)?.[1];
    expect(file?.workflow).toEqual(FILE.workflow);
    expect(file?.["last_number"]).toBe(9);
    expect(savedTemplates(work)).toEqual([
      BUG,
      HANDOFF,
      {
        name: "Release checklist",
        state: "next",
        priority: 1,
        labels: ["release", "web"],
        project: "p1",
        repeat: "2w",
        body: "Tag the build",
      },
    ]);
    expect(await screen.findByText("Saved template Release checklist.")).toBeDefined();
  });

  it("refuses a duplicate name before saving", async () => {
    const work = fakeApi({ teamSettings: vi.fn(() => Promise.resolve(FILE)) });
    const user = renderScreen("/issues/templates", work, fakePlanning());
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const form = screen.getByRole("form", { name: "New template" });
    await user.type(within(form).getByLabelText("Template name"), "bug report");
    await user.click(within(form).getByRole("button", { name: "Save template" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Another template already has this name.");
    expect(work.saveTeamSettings).not.toHaveBeenCalled();
  });

  it("edits, duplicates, reorders and removes", async () => {
    const work = fakeApi({ teamSettings: vi.fn(() => Promise.resolve(FILE)) });
    const user = renderScreen("/issues/templates", work, fakePlanning());
    await user.click(await screen.findByRole("button", { name: "Edit Bug report" }));
    const form = screen.getByRole("form", { name: "Edit template Bug report" });
    const title = within(form).getByLabelText("Title");
    await user.clear(title);
    await user.type(title, "Defect:");
    await user.click(within(form).getByRole("button", { name: "Save template" }));
    await waitFor(() => {
      expect(savedTemplates(work)?.[0]).toEqual({ ...BUG, title: "Defect:" });
    });

    await user.click(await screen.findByRole("button", { name: "Duplicate Bug report" }));
    await waitFor(() => {
      expect(savedTemplates(work)?.map((each) => each.name)).toEqual([
        "Bug report",
        "Bug report copy",
        "On-call handoff",
      ]);
    });

    await user.click(await screen.findByRole("button", { name: "Move On-call handoff up" }));
    await waitFor(() => {
      expect(savedTemplates(work)?.map((each) => each.name)).toEqual([
        "On-call handoff",
        "Bug report",
      ]);
    });

    await user.click(await screen.findByRole("button", { name: "Remove Bug report" }));
    await user.click(screen.getByRole("button", { name: "Remove Bug report" }));
    await waitFor(() => {
      expect(savedTemplates(work)).toEqual([HANDOFF]);
    });
  });

  it("shows the server's refusal and keeps the editor open", async () => {
    const work = fakeApi({
      teamSettings: vi.fn(() => Promise.resolve(FILE)),
      saveTeamSettings: vi.fn(() => Promise.reject(new Error("unknown status gone"))),
    });
    const user = renderScreen("/issues/templates", work, fakePlanning());
    await user.click(await screen.findByRole("button", { name: "Edit Bug report" }));
    const form = screen.getByRole("form", { name: "Edit template Bug report" });
    await user.click(within(form).getByRole("button", { name: "Save template" }));
    const alert = await within(form).findByRole("alert");
    expect(alert.textContent).toContain("unknown status gone");
  });

  it("shows repeating issues only once the templates have loaded", async () => {
    // Regression: the section rendered first and jumped down the page.
    const settle: { resolve?: (file: TeamFile) => void } = {};
    const work = fakeApi({
      teamSettings: vi.fn(
        () =>
          new Promise<TeamFile>((resolve) => {
            settle.resolve = resolve;
          }),
      ),
    });
    const planning = fakePlanning();
    renderScreen("/issues/templates", work, planning);
    await waitFor(() => {
      expect(work.teamSettings).toHaveBeenCalled();
    });
    expect(screen.queryByRole("link", { name: "ENG-4" })).toBeNull();
    settle.resolve?.(FILE);
    expect(await screen.findByRole("link", { name: "ENG-4" })).toBeDefined();
  });

  it("lists repeating issues and changes or stops their repeat", async () => {
    const work = fakeApi({ teamSettings: vi.fn(() => Promise.resolve(FILE)) });
    const planning = fakePlanning();
    const user = renderScreen("/issues/templates", work, planning);
    expect(await screen.findByRole("link", { name: "ENG-4" })).toBeDefined();
    expect(screen.getByText("Due 2026-11-02")).toBeDefined();
    await user.selectOptions(screen.getByLabelText("How often ENG-4 repeats"), "1m");
    expect(planning.setRepeat).toHaveBeenCalledWith("ENG-4", "1m");
    await user.click(await screen.findByRole("button", { name: "Stop repeating ENG-4" }));
    expect(planning.setRepeat).toHaveBeenLastCalledWith("ENG-4", "");
    expect(await screen.findByText("ENG-4 no longer repeats.")).toBeDefined();
  });
});

describe("issue page repeat", () => {
  it("sets a due date and a repeat together", async () => {
    const work = fakeApi({ issue: vi.fn(() => Promise.resolve(issue("ENG-9", "Rotate"))) });
    const planning = fakePlanning();
    const user = renderScreen("/issues/ENG-9", work, planning);
    const form = await screen.findByRole("form", { name: "Repeat" });
    const save = within(form).getByRole("button", { name: "Save repeat" });
    expect((save as HTMLButtonElement).disabled).toBe(true);
    await user.selectOptions(within(form).getByLabelText("Repeat"), "2w");
    await user.click(save);
    const alert = await within(form).findByRole("alert");
    expect(alert.textContent).toContain("Set a due date first.");
    expect(planning.setRepeat).not.toHaveBeenCalled();
    await user.type(within(form).getByLabelText("Due date"), "2026-11-02");
    await user.click(save);
    await waitFor(() => {
      expect(planning.setRepeat).toHaveBeenCalledWith("ENG-9", "2w", "2026-11-02");
    });
  });

  it("only shows the repeat without a server's planning API", async () => {
    const work = fakeApi({
      issue: vi.fn(() =>
        Promise.resolve({ ...issue("ENG-9", "Rotate"), due: "2026-11-02", repeat: "1w" }),
      ),
    });
    renderScreen("/issues/ENG-9", work);
    expect(await screen.findByText("Repeats every 1w")).toBeDefined();
    expect(screen.queryByRole("form", { name: "Repeat" })).toBeNull();
  });
});
