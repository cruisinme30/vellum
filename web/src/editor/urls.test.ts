// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { embedTarget, safeUrl } from "./urls";

describe("safeUrl", () => {
  it("keeps http(s) URLs, normalised", () => {
    expect(safeUrl(" https://example.com ")).toBe("https://example.com/");
    expect(safeUrl("http://example.com/a?b=c")).toBe("http://example.com/a?b=c");
  });

  it("keeps workspace-relative paths as written", () => {
    expect(safeUrl("/pages/abc")).toBe("/pages/abc");
    expect(safeUrl("assets/logo.png")).toBe("assets/logo.png");
  });

  it.each([
    "",
    "   ",
    "javascript:alert(1)",
    "data:text/html,hi",
    "//evil.example/x",
    "ftp://example.com",
    "not a url",
  ])("rejects %j", (value) => {
    expect(safeUrl(value)).toBeUndefined();
  });
});

describe("embedTarget", () => {
  it.each([
    [
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "YouTube",
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
    ],
    [
      "https://youtu.be/dQw4w9WgXcQ",
      "YouTube",
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
    ],
    ["https://www.loom.com/share/abc123", "Loom", "https://www.loom.com/embed/abc123"],
    [
      "https://www.figma.com/design/KEY/Name",
      "Figma",
      `https://www.figma.com/embed?embed_host=vellum&url=${encodeURIComponent("https://www.figma.com/design/KEY/Name")}`,
    ],
    [
      "https://codepen.io/someone/pen/xyz",
      "CodePen",
      "https://codepen.io/someone/embed/xyz?default-tab=result",
    ],
    [
      "https://gist.github.com/someone/0123",
      "GitHub Gist",
      "https://gist.github.com/someone/0123.pibb",
    ],
    [
      "https://www.google.com/maps/embed?pb=1",
      "Google Maps",
      "https://www.google.com/maps/embed?pb=1",
    ],
  ])("embeds %s", (url, provider, source) => {
    expect(embedTarget(url)).toEqual({ provider, src: source });
  });

  it.each([
    "not a url",
    // eslint-disable-next-line sonarjs/no-clear-text-protocols -- plain http must not embed.
    "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
    "https://www.youtube.com/watch",
    "https://www.youtube.com/watch?v=bad",
    "https://www.loom.com/share",
    "https://www.loom.com/other/abc",
    "https://www.figma.com/community/file",
    "https://www.figma.com/",
    "https://codepen.io/someone/details/xyz",
    "https://codepen.io/someone/pen",
    "https://gist.github.com/someone",
    "https://www.google.com/maps/place/x",
    "https://example.com/video",
  ])("does not embed %s", (url) => {
    expect(embedTarget(url)).toBeUndefined();
  });
});
