// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor, within } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../../api/http";

import { createHttpPlanningApi } from "./api";
import { fakeApi, GATEWAY, issue, renderAt } from "./fixtures";
import { InitiativesPage } from "./initiatives-page";
import { ProjectPage } from "./project-page";
import { inStatus, ProjectsPage } from "./projects-page";

describe("planning API", () => {
  it("maps operations to routes and drops empty fields", async () => {
    const get = vi.fn((path: string) =>
      Promise.resolve(
        path.endsWith("/issues")
          ? { groups: [{ issues: [issue("ENG-1", "a")] }] }
          : {
              projects: [],
              updates: [],
              initiatives: [],
              templates: [],
              forms: [],
              teams: [],
            },
      ),
    );
    const send = vi.fn((_method: string, path: string) =>
      Promise.resolve(
        path.endsWith("/issues") ? { issue: issue("ENG-2", "b") } : {},
      ),
    );
    const http = { get, send, forgetSession: vi.fn() } as unknown as Http;
    const api = createHttpPlanningApi(http);

    await expect(api.projectIssues("p 1")).resolves.toHaveLength(1);
    expect(get).toHaveBeenLastCalledWith("/api/v1/work/issues", {
      filter: '{"field":"project","op":"is","values":["p 1"]}',
      group_by: "none",
      order_by: "priority",
      show_closed: "past_month",
    });
    await api.updates("p 1");
    expect(get).toHaveBeenLastCalledWith("/api/v1/work/projects/p%201/updates");
    await api.createIssue({
      team: "ENG",
      title: "b",
      labels: [],
      due: "",
      repeat: "1w",
    });
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/work/issues", {
      team: "ENG",
      title: "b",
      repeat: "1w",
    });
    await api.postUpdate("p1", "on-track", "Good");
    expect(send).toHaveBeenLastCalledWith(
      "POST",
      "/api/v1/work/projects/p1/updates",
      {
        health: "on-track",
        body: "Good",
      },
    );
    await api.dismissSuggestion("ENG-3");
    expect(send).toHaveBeenLastCalledWith(
      "DELETE",
      "/api/v1/work/issues/ENG-3/triage/suggestion",
    );
  });
});

describe("projects", () => {
  it("lists active projects with health and progress, and filters by status", async () => {
    const user = renderAt(
      "/projects",
      fakeApi(),
      <Route path="/projects" element={<ProjectsPage />} />,
    );
    const title = await screen.findByText("Gateway replay");
    const row = title.closest("li");
    expect(row).not.toBeNull();
    const scoped = within(row as HTMLElement);
    expect(scoped.getByText("At risk")).toBeTruthy();
    expect(
      scoped.getByRole("progressbar", { name: "Progress of Gateway replay" }),
    ).toHaveProperty("value", 25);
    expect(scoped.getByText(/update due/)).toBeTruthy();
    expect(screen.queryByText("Old docs")).toBeNull();
    await user.selectOptions(screen.getByLabelText("Show"), "completed");
    expect(screen.getByText("Old docs")).toBeTruthy();
    expect(screen.queryByText("Gateway replay")).toBeNull();
  });

  it("creates a project and opens it", async () => {
    const api = fakeApi();
    const user = renderAt(
      "/projects",
      api,
      <>
        <Route path="/projects" element={<ProjectsPage />} />
        <Route path="/projects/:projectId" element={<p>opened</p>} />
      </>,
    );
    await user.click(await screen.findByText("New project"));
    await user.type(screen.getByLabelText("Title"), "Search v2");
    await user.type(screen.getByLabelText("Lead"), "@dev");
    await user.click(screen.getByRole("button", { name: "Create project" }));
    expect(await screen.findByText("opened")).toBeTruthy();
    expect(api.createProject).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Search v2",
        teams: ["ENG"],
        lead: "@dev",
        status: "planned",
      }),
    );
  });

  it("filters statuses", () => {
    expect(inStatus(GATEWAY, "active")).toBe(true);
    expect(inStatus(GATEWAY, "completed")).toBe(false);
    expect(inStatus(GATEWAY, "all")).toBe(true);
  });

  it("shows milestones, posts an update and moves an issue between milestones", async () => {
    const api = fakeApi();
    const user = renderAt(
      "/projects/p1",
      api,
      <Route path="/projects/:projectId" element={<ProjectPage />} />,
    );
    expect(
      await screen.findByRole("heading", { name: "Gateway replay", level: 1 }),
    ).toBeTruthy();
    expect(
      screen.getByText("An update is due. Say how the project is going."),
    ).toBeTruthy();
    expect(
      screen.getByRole("progressbar", { name: "Progress of Beta" }),
    ).toHaveProperty("value", 50);
    expect(await screen.findByText("Replay is slow.")).toBeTruthy();
    expect(screen.getByText(/Target date moved/)).toBeTruthy();

    await user.click(screen.getByRole("radio", { name: "Off track" }));
    await user.type(
      screen.getByLabelText(/What changed/),
      "Blocked on storage.",
    );
    await user.click(screen.getByRole("button", { name: "Post update" }));
    await waitFor(() => {
      expect(api.postUpdate).toHaveBeenCalledWith(
        "p1",
        "off-track",
        "Blocked on storage.",
      );
    });

    await user.type(screen.getByLabelText("Milestone"), "GA");
    await user.click(screen.getByRole("button", { name: "Add milestone" }));
    await waitFor(() => {
      expect(api.addMilestone).toHaveBeenCalledWith("p1", "GA", "");
    });

    await user.selectOptions(
      await screen.findByLabelText("Milestone of ENG-1"),
      "",
    );
    await waitFor(() => {
      expect(api.setMilestone).toHaveBeenCalledWith("ENG-1", "");
    });
    expect(
      screen.getByRole("link", { name: "New issue" }).getAttribute("href"),
    ).toBe("/issues/new?team=ENG&project=p1");
  });

  it("rolls initiatives up", async () => {
    renderAt(
      "/initiatives",
      fakeApi(),
      <Route path="/initiatives" element={<InitiativesPage />} />,
    );
    const heading = await screen.findByRole("heading", { name: "Q4 launch" });
    const card = within(heading.closest("li") as HTMLElement);
    expect(card.getAllByText("At risk")).toHaveLength(2);
    expect(card.getByText("Gateway replay")).toBeTruthy();
  });

  it("needs a server", () => {
    renderAt(
      "/projects",
      undefined,
      <Route path="/projects" element={<ProjectsPage />} />,
    );
    expect(screen.getByText(/needs a Vellum server/i)).toBeTruthy();
  });
});
