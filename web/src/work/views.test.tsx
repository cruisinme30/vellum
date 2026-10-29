// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/* eslint-disable @typescript-eslint/unbound-method -- the API's methods are vi.fn mocks */

import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { fakeApi, issue, renderAt, view, where } from "../test/work-fixtures";

import { createHttpWorkApi, type GroupOut } from "./api";
import { EVERYTHING, newCondition, newGroup, parse, serialize } from "./filter";
import { IssuesPage } from "./issues-page";

describe("filters", () => {
  it("serialises AND/OR trees and reads them back", () => {
    const tree = newGroup("and", [
      { ...newCondition("priority"), values: ["urgent", "high"] },
      newGroup("or", [
        newCondition("assignee"),
        { ...newCondition("label"), op: "includes_all", values: ["gateway"] },
      ]),
      newCondition("title"),
    ]);
    const text = serialize(tree);
    expect(JSON.parse(text ?? "")).toEqual({
      and: [
        { field: "priority", op: "is", values: ["urgent", "high"] },
        {
          or: [
            { field: "assignee", op: "is", values: ["@me"] },
            { field: "label", op: "includes_all", values: ["gateway"] },
          ],
        },
      ],
    });
    expect(serialize(parse(text) ?? EVERYTHING)).toBe(text);
    expect(serialize(EVERYTHING)).toBeUndefined();
    expect(parse('{"not":{"field":"title","op":"contains","values":["x"]}}')).toBeUndefined();
    expect(parse("{oops")).toBeUndefined();
    expect(parse('{"field":"priority","op":"lt","values":["1"]}')).toBeUndefined();
  });
});

describe("work API views, triage and cycles", () => {
  it("calls the routes", async () => {
    const get = vi.fn((path: string) =>
      Promise.resolve(
        path.endsWith("/inbox") || path.endsWith("/issues")
          ? { groups: [{ issues: [issue("ENG-1", "a")] }] }
          : { views: [view()], cycles: [], issues: [] },
      ),
    );
    const send = vi.fn(() => Promise.resolve(view()));
    const api = createHttpWorkApi({ get, send } as unknown as Http);
    await api.queryIssues({ team: "ENG", group_by: "status" });
    expect(get).toHaveBeenCalledWith("/api/v1/work/issues", { team: "ENG", group_by: "status" });
    expect(await api.inbox("ENG")).toHaveLength(1);
    expect(get).toHaveBeenCalledWith("/api/v1/work/teams/ENG/inbox");
    await api.triage("ENG-1", { action: "duplicate", duplicate_of: "ENG-2" });
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/work/issues/ENG-1/triage", {
      action: "duplicate",
      duplicate_of: "ENG-2",
    });
    await api.cycles("ENG");
    expect(get).toHaveBeenCalledWith("/api/v1/work/teams/ENG/cycles");
    await api.myIssues("created");
    expect(get).toHaveBeenCalledWith("/api/v1/work/me/issues", { tab: "created" });
    expect(await api.views("ENG")).toHaveLength(1);
    await api.createView({ name: "x" });
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/work/views", { name: "x" });
    await api.updateView("v/1", { name: "y" });
    expect(send).toHaveBeenCalledWith("PUT", "/api/v1/work/views/v%2F1", { name: "y" });
    await api.deleteView("v1");
    expect(send).toHaveBeenCalledWith("DELETE", "/api/v1/work/views/v1");
    await api.setPriority("ENG-1", 2);
    expect(send).toHaveBeenCalledWith("PATCH", "/api/v1/work/issues/ENG-1", { priority: 2 });
  });
});

describe("issue views", () => {
  it("shows a board by status and moves cards from the keyboard", async () => {
    const api = fakeApi();
    renderAt("/issues?layout=board", api, <Route path="issues" element={<IssuesPage />} />);
    const todo = await screen.findByRole("region", { name: "Todo" });
    expect(screen.getByRole("region", { name: "Done" })).toBeDefined();
    const card = within(todo).getByRole("link", { name: /ENG-1/ });
    expect(card.textContent).toContain("Urgent");
    expect(api.queryIssues).toHaveBeenCalledWith(
      expect.objectContaining({ team: "ENG", group_by: "status", layout: "board" }),
    );
    card.focus();
    fireEvent.keyDown(card, { key: "ArrowRight", shiftKey: true });
    await waitFor(() => {
      expect(api.setState).toHaveBeenCalledWith("ENG-1", "doing");
    });
    await waitFor(() => {
      expect(document.querySelector(".vl-results [aria-live]")?.textContent).toBe(
        "Moved ENG-1 to In Progress.",
      );
    });
  });

  it("moves the focus with J/K and L", async () => {
    renderAt("/issues?layout=board", fakeApi(), <Route path="issues" element={<IssuesPage />} />);
    const first = await screen.findByRole("link", { name: /ENG-1/ });
    first.focus();
    fireEvent.keyDown(first, { key: "j" });
    expect(document.activeElement?.textContent).toContain("ENG-2");
    fireEvent.keyDown(document.activeElement ?? first, { key: "k" });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: "l" });
    expect(document.activeElement?.textContent).toContain("ENG-2");
  });

  it("drops a card into another priority column", async () => {
    const groups: GroupOut[] = [
      { key: "urgent", issues: [issue("ENG-1", "a", { priority: 1 })] },
      { key: "low", issues: [] },
    ];
    const api = fakeApi({ queryIssues: vi.fn(() => Promise.resolve(groups)) });
    renderAt(
      "/issues?layout=board&group=priority",
      api,
      <Route path="issues" element={<IssuesPage />} />,
    );
    const card = await screen.findByRole("link", { name: /ENG-1/ });
    const data = new Map<string, string>();
    const dataTransfer = {
      setData: (type: string, value: string) => data.set(type, value),
      getData: (type: string) => data.get(type) ?? "",
      effectAllowed: "",
    };
    fireEvent.dragStart(card, { dataTransfer });
    fireEvent.drop(screen.getByRole("region", { name: "Low" }), { dataTransfer });
    await waitFor(() => {
      expect(api.setPriority).toHaveBeenCalledWith("ENG-1", 4);
    });
  });

  it("builds a filter and puts it in the address", async () => {
    const api = fakeApi();
    const user = renderAt("/issues", api, <Route path="issues" element={<IssuesPage />} />);
    await user.click(await screen.findByRole("button", { name: "Filter" }));
    await user.click(screen.getByRole("button", { name: "Add condition" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Field" }), "priority");
    await user.type(screen.getByRole("combobox", { name: "Values" }), "urgent, high");
    await user.selectOptions(screen.getByRole("combobox", { name: "Match" }), "or");
    await user.click(screen.getByRole("button", { name: "Apply filter" }));
    const filter = '{"or":[{"field":"priority","op":"is","values":["urgent","high"]}]}';
    await waitFor(() => {
      expect(api.queryIssues).toHaveBeenLastCalledWith(expect.objectContaining({ filter }));
    });
    expect(where()).toContain(`filter=${filter}`);
    expect(screen.getByRole("button", { name: "Filter (1 condition)" })).toBeDefined();
  });

  it("opens, saves and deletes saved views", async () => {
    const api = fakeApi({ views: vi.fn(() => Promise.resolve([view()])) });
    const user = renderAt("/issues", api, <Route path="issues" element={<IssuesPage />} />);
    const picker = await screen.findByRole("combobox", { name: "Saved view" });
    await within(picker).findByRole("option", { name: "Urgent board (shared by dev)" });
    await user.selectOptions(picker, "v1");
    await waitFor(() => {
      expect(where()).toContain("group=priority");
    });
    expect(where()).toContain("layout=board");
    expect(where()).toContain("view=v1");
    expect(api.queryIssues).toHaveBeenLastCalledWith(
      expect.objectContaining({ group_by: "priority", order_by: "due", show_closed: "none" }),
    );

    await user.click(screen.getByRole("button", { name: "Save as view" }));
    await user.type(screen.getByRole("textbox", { name: "View name" }), "Mine");
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Who can open it" }),
      "workspace",
    );
    await user.click(screen.getByRole("button", { name: "Save view" }));
    expect(api.createView).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Mine",
        team: "ENG",
        visibility: "workspace",
        group_by: "priority",
      }),
    );
  });

  it("goes to triage with the G T chord", async () => {
    renderAt("/issues?team=ENG", fakeApi(), <Route path="issues" element={<IssuesPage />} />);
    await screen.findByRole("link", { name: /ENG-1/ });
    fireEvent.keyDown(document.body, { key: "g" });
    fireEvent.keyDown(document.body, { key: "t" });
    await waitFor(() => {
      expect(where()).toBe("/issues/triage?team=ENG");
    });
  });

  it("lists the team's statuses by category", async () => {
    renderAt("/issues", fakeApi(), <Route path="issues" element={<IssuesPage />} />);
    const statuses = await screen.findByText("Engineering statuses");
    expect(statuses.closest("details")?.textContent).toContain("Started");
    expect(statuses.closest("details")?.textContent).toContain("Tododefault");
  });
});
