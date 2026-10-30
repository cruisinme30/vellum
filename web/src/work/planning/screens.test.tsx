// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { AskPage } from "./asks-page";
import { fakeApi, NONE, renderAt } from "./fixtures";
import { duration, InsightsPage } from "./insights-page";
import { draftFromUrl, NewIssuePage } from "./new-issue-page";
import { TriageSuggestion } from "./suggestion";

describe("insights", () => {
  it("asks again when the breakdown changes and shows a table", async () => {
    const api = fakeApi();
    const user = renderAt(
      "/insights",
      api,
      <Route path="/insights" element={<InsightsPage />} />,
    );
    expect(await screen.findByText("12 issues")).toBeTruthy();
    expect(screen.getByText("Median 30 h")).toBeTruthy();
    expect(screen.getByText("85% within 4 d")).toBeTruthy();
    await user.selectOptions(screen.getByLabelText("Break down by"), "team");
    expect(await screen.findByRole("rowheader", { name: "ENG" })).toBeTruthy();
    expect(screen.getByRole("rowheader", { name: "(none)" })).toBeTruthy();
    expect(api.insights).toHaveBeenLastCalledWith(
      expect.objectContaining({ pivot: "team", measure: "lead_time" }),
    );
  });

  it("writes durations for people", () => {
    expect(duration(5)).toBe("5 h");
    expect(duration(72)).toBe("3 d");
    expect(duration()).toBe("–");
  });
});

describe("asks", () => {
  it("renders each field kind and files the answers", async () => {
    const api = fakeApi();
    const user = renderAt(
      "/asks/ENG/bug",
      api,
      <Route path="/asks/:team/:slug" element={<AskPage />} />,
    );
    expect(
      await screen.findByRole("heading", { name: "Report a bug" }),
    ).toBeTruthy();
    await user.type(
      screen.getByLabelText("Summary (required)"),
      "Crash on save",
    );
    await user.selectOptions(screen.getByLabelText("Area"), "Web");
    await user.type(screen.getByLabelText("Details"), "Every time.");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(
      await screen.findByText("Sent as ENG-10. The team will triage it."),
    ).toBeTruthy();
    expect(api.submitForm).toHaveBeenCalledWith("ENG", "bug", {
      summary: "Crash on save",
      area: "Web",
      details: "Every time.",
    });
  });
});

describe("new issue", () => {
  it("reads a create URL", () => {
    const draft = draftFromUrl(
      new URLSearchParams("team=ENG&title=Fix+login&labels=bug,web&evil=1"),
    );
    expect(draft).toEqual({
      team: "ENG",
      title: "Fix login",
      labels: "bug,web",
    });
  });

  it("prefills from the URL, previews the template and sends every field", async () => {
    const api = fakeApi();
    const user = renderAt(
      "/issues/new?team=ENG&title=Rotate+keys&labels=ops&due=2026-11-02&repeat=2w&template=Bug",
      api,
      <Route path="/issues/new" element={<NewIssuePage />} />,
    );
    expect(await screen.findByDisplayValue("Rotate keys")).toBeTruthy();
    expect(await screen.findByText("Steps to reproduce:")).toBeTruthy();
    expect(screen.getByText("Repeats every 1w")).toBeTruthy();
    expect(screen.getByLabelText("Repeat")).toHaveProperty("value", "2w");
    await user.selectOptions(screen.getByLabelText("Priority"), "1");
    await user.click(screen.getByRole("button", { name: "Create issue" }));
    expect(await screen.findByText("issue page")).toBeTruthy();
    expect(api.createIssue).toHaveBeenCalledWith(
      expect.objectContaining({
        team: "ENG",
        title: "Rotate keys",
        template: "Bug",
        priority: 1,
        labels: ["ops"],
        due: "2026-11-02",
        repeat: "2w",
      }),
    );
  });
});

describe("triage suggestion", () => {
  it("shows the waiting suggestion and accepts it", async () => {
    const api = fakeApi();
    const applied = vi.fn();
    const user = renderAt(
      "/",
      api,
      <Route
        path="/"
        element={<TriageSuggestion issueKey="ENG-3" onApplied={applied} />}
      />,
    );
    expect(
      await screen.findByRole("heading", { name: "Claude suggests" }),
    ).toBeTruthy();
    expect(screen.getByText("Move to OPS")).toBeTruthy();
    expect(screen.getByText("Priority Urgent")).toBeTruthy();
    expect(screen.getByText("Looks like the pager outage.")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Accept" }));
    await waitFor(() => {
      expect(applied).toHaveBeenCalled();
    });
    expect(api.acceptSuggestion).toHaveBeenCalledWith("ENG-3");
  });

  it("shows nothing when no suggestion waits", async () => {
    const api = fakeApi({
      brief: vi.fn(() =>
        Promise.resolve({
          key: "ENG-3",
          title: "",
          body: "",
          team: "ENG",
          teams: [],
          similar: [],
          suggestion: NONE,
        }),
      ),
    });
    renderAt(
      "/",
      api,
      <Route path="/" element={<TriageSuggestion issueKey="ENG-3" />} />,
    );
    await waitFor(() => {
      expect(api.brief).toHaveBeenCalled();
    });
    expect(screen.queryByRole("button", { name: "Accept" })).toBeNull();
  });
});
