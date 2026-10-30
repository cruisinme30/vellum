// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";

import {
  createHttpWorkApi,
  issueChipStatus,
  type IssueOut,
  type TeamOut,
  type WorkApi,
} from "./api";

const ISSUE = { key: "ENG-7", title: "Fix sign-in", team: "ENG", state: "todo" } as IssueOut;

const offline = () => Promise.reject(new Error("offline"));

interface Call {
  readonly method: string;
  readonly path: string;
  readonly query?: unknown;
}

/** An Http that records each call and answers from `answer`. */
function fakeHttp(answer: (method: string, path: string) => unknown = () => ({})) {
  const calls: Call[] = [];
  const reply = (method: string, path: string) => {
    try {
      return Promise.resolve(answer(method, path));
    } catch (error) {
      return Promise.reject(error instanceof Error ? error : new Error(String(error)));
    }
  };
  const http = {
    get: (path: string, query?: unknown) => {
      calls.push({ method: "GET", path, query });
      return reply("GET", path);
    },
    send: (method: string, path: string) => {
      calls.push({ method, path });
      return reply(method, path);
    },
    upload: () => Promise.reject(new Error("upload is not used")),
  } as unknown as Http;
  return { http, calls };
}

describe("the Work API over HTTP", () => {
  it("completes a task, or reopens it", async () => {
    const { http, calls } = fakeHttp((_, path) =>
      path.endsWith("/complete") ? { task: { id: "k1" } } : { id: "k1" },
    );
    const api = createHttpWorkApi(http);
    await expect(api.setDone("k1", true)).resolves.toEqual({ id: "k1" });
    await expect(api.setDone("k1", false)).resolves.toEqual({ id: "k1" });
    expect(calls.map((call) => call.path)).toEqual([
      "/api/v1/tasks/k1/complete",
      "/api/v1/tasks/k1/reopen",
    ]);
  });

  it("lists teams, with archived ones only when asked", async () => {
    const { http, calls } = fakeHttp(() => ({ teams: [] }));
    const api = createHttpWorkApi(http);
    await api.teams();
    await api.teams({ includeArchived: true });
    expect(calls.map((call) => call.query)).toEqual([undefined, { include_archived: "true" }]);
  });

  it("looks up an issue key first, and falls back to titles", async () => {
    let known = true;
    const { http, calls } = fakeHttp((_, path) => {
      if (path.endsWith("/issues/ENG-7")) {
        if (!known) throw new Error("not found");
        return ISSUE;
      }
      return { groups: [{ issues: [ISSUE] }] };
    });
    const api = createHttpWorkApi(http);
    await expect(api.searchIssues("eng-7")).resolves.toEqual([ISSUE]);
    known = false;
    await expect(api.searchIssues("eng-7")).resolves.toEqual([ISSUE]);
    await expect(api.searchIssues("sign")).resolves.toEqual([ISSUE]);
    expect(calls.filter((call) => call.path.endsWith("/issues"))).toHaveLength(2);
  });

  it("scopes saved views to a team only when one is given", async () => {
    const { http, calls } = fakeHttp(() => ({ views: [] }));
    const api = createHttpWorkApi(http);
    await api.views();
    await api.views("ENG");
    expect(calls.map((call) => call.query)).toEqual([undefined, { team: "ENG" }]);
  });

  it("archives and restores a team", async () => {
    const { http, calls } = fakeHttp();
    const api = createHttpWorkApi(http);
    await api.setTeamArchived("ENG", true);
    await api.setTeamArchived("ENG", false);
    expect(calls.map((call) => call.path)).toEqual([
      "/api/v1/work/teams/ENG/archive",
      "/api/v1/work/teams/ENG/restore",
    ]);
  });

  it("returns the changed thread, or nothing when the server sends none", async () => {
    let answer: object = { thread: { id: "t1" } };
    const { http } = fakeHttp(() => answer);
    const api = createHttpWorkApi(http);
    await expect(api.reply("c1", "Thanks")).resolves.toEqual({ id: "t1" });
    answer = {};
    await expect(api.resolve("t1", true)).resolves.toBeUndefined();
  });
});

describe("issue chip status", () => {
  const team = {
    key: "ENG",
    statuses: [{ key: "todo", name: "Up next", category: "unstarted" }],
  } as TeamOut;
  const issues = new Map<string, IssueOut>([
    ["ENG-7", ISSUE],
    ["ENG-8", { ...ISSUE, key: "ENG-8", state: "shipped", category: "completed" }],
    ["OPS-1", { ...ISSUE, key: "OPS-1", team: "OPS", category: "canceled" }],
  ]);
  const issue = (key: string) => {
    const found = issues.get(key);
    return found === undefined ? Promise.reject(new Error(key)) : Promise.resolve(found);
  };

  it("names states the way the team does, and says when an issue is finished", async () => {
    const teams = vi.fn(() => Promise.resolve([team]));
    const status = issueChipStatus({ teams, issue } as unknown as WorkApi);
    await expect(status("ENG-7")).resolves.toEqual({
      title: "Fix sign-in",
      state: "Up next",
      closed: false,
    });
    await expect(status("ENG-8")).resolves.toEqual({
      title: "Fix sign-in",
      state: "shipped",
      closed: true,
    });
    await expect(status("OPS-1")).resolves.toMatchObject({ closed: true });
    expect(teams).toHaveBeenCalledTimes(1);
  });

  it("shows raw state keys when the teams cannot be read", async () => {
    const status = issueChipStatus({ teams: offline, issue } as unknown as WorkApi);
    await expect(status("ENG-7")).resolves.toMatchObject({ state: "todo" });
  });
});
