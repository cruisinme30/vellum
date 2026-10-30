// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { renderWithProviders } from "../test/render-app";

import { HelpLink, ViewTitle, publishedUrl, serverUrl } from "./help-link";
import { HELP_TOPICS, type HelpTopic } from "./topics.gen";

describe("contextual help", () => {
  it("maps every topic to the published guide", () => {
    for (const topic of Object.keys(HELP_TOPICS) as HelpTopic[]) {
      const url = publishedUrl(topic);
      expect(url).toMatch(
        /^https:\/\/cruisinme30\.github\.io\/vellum\/docs\/(user|admin|developer)\//,
      );
      expect(url).not.toMatch(/\.md(#|$)/);
    }
    expect(publishedUrl("editor.slash-menu")).toBe(
      "https://cruisinme30.github.io/vellum/docs/user/start/index.html#slash-menu",
    );
    expect(publishedUrl("admin.overview")).toBe(
      "https://cruisinme30.github.io/vellum/docs/admin/index.html",
    );
    expect(serverUrl("mywork.buckets")).toBe("/guide/topics/mywork.buckets");
  });

  it("opens the topic in a new tab with a name that says what it explains", () => {
    renderWithProviders(<HelpLink topic="collection.views" label="Roadmap" />);
    const link = screen.getByRole("link", { name: "Help: Roadmap" });
    expect(link.getAttribute("href")).toBe(publishedUrl("collection.views"));
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("rel")).toBe("noreferrer");
  });

  it("keeps the heading's name to the title alone", () => {
    renderWithProviders(<ViewTitle title="Data" help="collection.create" />);
    expect(screen.getByRole("heading", { level: 1, name: "Data" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Help: Data" })).toBeTruthy();
  });
});
