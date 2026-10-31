// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";
import { describe, expect, it } from "vitest";

import { missingAttributes } from "./schema-attributes";
import { createTestEditor } from "./test-editor";

const paragraph = (text: string): JSONContent => ({
  type: "paragraph",
  content: [{ type: "text", text }],
});

/**
 * Loads content, writes it to HTML and reads that back, as copy and paste do.
 */
function roundTrip(content: JSONContent[]): JSONContent[] {
  const first = createTestEditor({ content: { type: "doc", content } });
  const second = createTestEditor({ content: first.getHTML() });
  const result = second.getJSON().content;
  first.destroy();
  second.destroy();
  return result;
}

describe("schema attributes on stock nodes", () => {
  it("adds only what Tiptap lacks", () => {
    const byType = new Map(
      missingAttributes().map((entry) => [
        entry.types[0],
        Object.keys(entry.attributes),
      ]),
    );
    expect(byType.get("heading")).toEqual(["attributes"]);
    expect(byType.get("bulletList")).toEqual(["tight"]);
    expect(byType.get("orderedList")).toEqual(["tight"]);
    expect(byType.get("taskList")).toEqual(["tight"]);
    expect(byType.get("taskItem")).toBeUndefined();
  });

  it("keeps a heading's attribute span and a loose list", () => {
    const [heading, list] = roundTrip([
      {
        type: "heading",
        attrs: { level: 2, attributes: "{#intro .lead}" },
        content: [{ type: "text", text: "Intro" }],
      },
      {
        type: "bulletList",
        attrs: { tight: false },
        content: [{ type: "listItem", content: [paragraph("One")] }],
      },
    ]);
    expect(heading?.attrs).toMatchObject({
      level: 2,
      attributes: "{#intro .lead}",
    });
    expect(list?.attrs).toMatchObject({ tight: false });
  });

  it("lets an ordered list hold tasks", () => {
    const [list] = roundTrip([
      {
        type: "orderedList",
        content: [
          { type: "listItem", content: [paragraph("Plain")] },
          {
            type: "taskItem",
            attrs: { checked: true },
            content: [paragraph("Done")],
          },
        ],
      },
    ]);
    expect(list?.content?.map((item) => item.type)).toEqual([
      "listItem",
      "taskItem",
    ]);
    expect(list?.content?.[1]?.attrs).toMatchObject({ checked: true });
  });

  it("keeps a code block's info string", () => {
    const [code] = roundTrip([
      {
        type: "codeBlock",
        attrs: {
          language: "rust",
          args: "title=main.rs",
          attributes: "{.wide}",
        },
        content: [{ type: "text", text: "fn main() {}" }],
      },
    ]);
    expect(code?.attrs).toMatchObject({
      language: "rust",
      args: "title=main.rs",
      attributes: "{.wide}",
    });
  });

  it("keeps an unknown container and a callout title", () => {
    const [container, callout] = roundTrip([
      {
        type: "container",
        attrs: { name: "aside", arg: "left", attributes: "{.muted}" },
        content: [paragraph("Body")],
      },
      {
        type: "callout",
        attrs: { kind: "warn", title: "Heads up" },
        content: [paragraph("Hi")],
      },
    ]);
    expect(container?.attrs).toMatchObject({
      name: "aside",
      arg: "left",
      attributes: "{.muted}",
    });
    expect(callout?.attrs).toMatchObject({ kind: "warn", title: "Heads up" });
  });
});

const image = (source: string): JSONContent => ({
  type: "paragraph",
  content: [
    { type: "text", text: "See " },
    {
      type: "inlineImage",
      attrs: { src: source, alt: "logo", title: "Our logo" },
    },
  ],
});

describe("inline images", () => {
  it("round-trips inside a line of text", () => {
    const [line] = roundTrip([image("/assets/logo.png")]);
    expect(line?.content?.[1]).toMatchObject({
      type: "inlineImage",
      attrs: { src: "/assets/logo.png", alt: "logo", title: "Our logo" },
    });
  });

  it("renders a safe image and its Markdown text", () => {
    const editor = createTestEditor({
      content: { type: "doc", content: [image("/a.png")] },
    });
    const element = editor.view.dom.querySelector("img.vl-inline-image");
    expect(element?.getAttribute("src")).toBe("/a.png");
    expect(element?.getAttribute("title")).toBe("Our logo");
    expect(editor.getText()).toBe("See ![logo](/a.png)");
    editor.destroy();
  });

  it("drops an unsafe source and an empty title", () => {
    const editor = createTestEditor({
      content: {
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [
              {
                type: "inlineImage",
                attrs: { src: "javascript:alert(1)", alt: "x" },
              },
            ],
          },
        ],
      },
    });
    const element = editor.view.dom.querySelector("img.vl-inline-image");
    expect(element?.hasAttribute("src")).toBe(false);
    expect(element?.hasAttribute("title")).toBe(false);
    editor.destroy();
  });
});
