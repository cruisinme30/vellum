// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor, within } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { createNorthstarStore } from "../data/mock/northstar";

import { createHttpWorkApi } from "./api";
import { IssuePage } from "./issue-page";
import { IssuesPage } from "./issues-page";
import { fakeApi, issue, NONE, renderAt, task } from "./test-work";

describe("work API", () => {
  it("calls the task and issue routes", async () => {
    const get = vi.fn((path: string) =>
      Promise.resolve(
        path === "/api/v1/work/issues"
          ? { groups: [{ issues: [issue("ENG-1", "a")] }, { issues: [issue("ENG-2", "b")] }] }
          : {
              threads: [
                {
                  comments: [
                    { id: "2", created_at: "2026-10-29T11:00:00Z", deleted: false },
                    { id: "x", created_at: "2026-10-29T09:00:00Z", deleted: true },
                  ],
                },
                { comments: [{ id: "1", created_at: "2026-10-29T10:00:00Z", deleted: false }] },
              ],
            },
      ),
    );
    const send = vi.fn((_method: string, path: string) =>
      Promise.resolve(path.endsWith("/complete") ? { task: task("k1", "a", { done: true }) } : {}),
    );
    const api = createHttpWorkApi({ get, send, forgetSession: vi.fn() } as unknown as Http);

    const issues = await api.issues("ENG");
    expect(issues.map((each) => each.key)).toEqual(["ENG-1", "ENG-2"]);
    expect(get).toHaveBeenLastCalledWith("/api/v1/work/issues", {
      team: "ENG",
      group_by: "none",
      order_by: "priority",
      show_closed: "none",
    });
    const comments = await api.comments("ENG-1");
    expect(comments.map((each) => each.id)).toEqual(["1", "2"]);
    expect(get).toHaveBeenLastCalledWith("/api/v1/work/comments", { target: "ENG-1" });

    const completed = await api.setDone("k1", true);
    expect(completed.done).toBe(true);
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/tasks/k1/complete", {});
    await api.setDone("k1", false);
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/tasks/k1/reopen", {});
    await api.dismissMention("mention.c9");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/my-work/mentions/c9/dismiss", {});
    await api.saveSettings({ time_zone: "Asia/Tokyo", day_starts_at: "09:00", digest_at: NONE });
    expect(send).toHaveBeenLastCalledWith("PUT", "/api/v1/my-work/settings", {
      time_zone: "Asia/Tokyo",
      day_starts_at: "09:00",
      digest_at: NONE,
    });
    await api.setState("ENG-1", "done");
    expect(send).toHaveBeenLastCalledWith("PATCH", "/api/v1/work/issues/ENG-1", {
      state: "done",
    });
  });
});

describe("issues", () => {
  it("lists a team's issues and files a new one", async () => {
    const { api, createIssue } = fakeApi();
    const user = renderAt(
      "/issues",
      api,
      <>
        <Route path="issues" element={<IssuesPage />} />
        <Route path="issues/:issueKey" element={<p>opened</p>} />
      </>,
    );
    const link = await screen.findByRole("link", { name: /ENG-7/ });
    expect(link.textContent).toContain("Todo");
    await user.type(
      screen.getByRole("textbox", { name: "New issue for Engineering" }),
      "Add dark mode",
    );
    await user.click(screen.getByRole("button", { name: "Create issue" }));
    expect(createIssue).toHaveBeenCalledWith({ team: "ENG", title: "Add dark mode" });
    expect(await screen.findByText("opened")).toBeDefined();
  });

  it("shows an issue, changes its state and adds a comment", async () => {
    const { api, setState, addComment } = fakeApi();
    const user = renderAt(
      "/issues/ENG-7",
      api,
      <Route path="issues/:issueKey" element={<IssuePage />} />,
    );
    expect(await screen.findByRole("heading", { level: 1, name: "Fix sign-in" })).toBeDefined();
    const comments = await screen.findByRole("list");
    expect(within(comments).getByText("Seen on Safari.")).toBeDefined();

    await user.selectOptions(screen.getByRole("combobox", { name: "State" }), "done");
    expect(setState).toHaveBeenCalledWith("ENG-7", "done");

    await user.type(screen.getByRole("textbox", { name: "Add a comment" }), "Fixed in 1.0.1");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    expect(addComment).toHaveBeenCalledWith("ENG-7", "Fixed in 1.0.1");

    const box = screen.getByRole("textbox", { name: "Add a comment" });
    await waitFor(() => {
      expect(box).toHaveProperty("value", "");
    });
    // user-event keeps its own copy of the value, which form.reset() does not update.
    await user.clear(box);
    await user.type(box, "Can you check?");
    await user.type(screen.getByRole("textbox", { name: "Assign to (optional)" }), "@sam");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    expect(addComment).toHaveBeenLastCalledWith("ENG-7", "Can you check?", "@sam");
  });

  it("starts a doc from an issue and opens it", async () => {
    const { api } = fakeApi();
    const store = createNorthstarStore();
    const createPage = vi.spyOn(store, "createPage");
    const user = renderAt(
      "/issues/ENG-7",
      api,
      <>
        <Route path="issues/:issueKey" element={<IssuePage />} />
        <Route path="p/:pageId" element={<p>opened page</p>} />
      </>,
      store,
    );
    await user.click(await screen.findByRole("button", { name: "Create doc from issue" }));
    expect(await screen.findByText("opened page")).toBeDefined();
    expect(createPage).toHaveBeenCalledWith(
      expect.objectContaining({ title: "ENG-7 Fix sign-in", body: "Issue: ENG-7\n" }),
    );
    const created = store.getSnapshot().pages.find((page) => page.title === "ENG-7 Fix sign-in");
    expect(created && store.seedFor(created.id)?.content?.[0]?.content?.[0]?.text).toBe(
      "Issue: ENG-7",
    );
  });

  it("shows a failure with its message", async () => {
    const { api } = fakeApi({ issue: () => Promise.reject(new Error("not found")) });
    renderAt("/issues/ENG-9", api, <Route path="issues/:issueKey" element={<IssuePage />} />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("not found");
  });
});
