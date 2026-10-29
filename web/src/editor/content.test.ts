// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";
import { describe, expect, it } from "vitest";

import { isBlockId } from "./block-id";
import {
  b,
  body,
  callout,
  code,
  codeBlock,
  columns,
  date,
  details,
  em,
  embed,
  fence,
  h,
  hl,
  hr,
  image,
  issue,
  link,
  linkCard,
  listBlock,
  marked,
  math,
  mathBlock,
  mention,
  ol,
  p,
  quote,
  table,
  tabs,
  tasks,
  toc,
  ul,
  wiki,
  withIds,
} from "./content";
import { NODES } from "./schema/blocks.gen";

const NODE_NAMES = new Set<string>(NODES.map((node) => node.name));

function types(node: JSONContent): string[] {
  return [node.type ?? "", ...(node.content ?? []).flatMap((child) => types(child))];
}

describe("inline builders", () => {
  it("builds marked text runs", () => {
    expect(b("x").marks).toEqual([{ type: "bold" }]);
    expect(em("x").marks).toEqual([{ type: "italic" }]);
    expect(code("x").marks).toEqual([{ type: "code" }]);
    expect(hl("x").marks).toEqual([{ type: "highlight" }]);
    expect(link("x", "https://a.example").marks).toEqual([
      { type: "link", attrs: { href: "https://a.example", title: "" } },
    ]);
    expect(marked("x", "bold", { type: "italic" })).toEqual({
      type: "text",
      text: "x",
      marks: [{ type: "bold" }, { type: "italic" }],
    });
  });

  it("builds inline atoms", () => {
    expect(mention("sam")).toEqual({ type: "mention", attrs: { handle: "sam" } });
    expect(wiki("Home")).toEqual({ type: "wikiLink", attrs: { target: "Home", label: "" } });
    expect(issue("ENG-1").attrs).toEqual({ key: "ENG-1" });
    expect(date("2026-01-01").attrs).toEqual({ date: "2026-01-01" });
    expect(math("x^2").attrs).toEqual({ literal: "x^2" });
  });
});

describe("block builders", () => {
  it("wraps strings in text nodes and omits empty paragraph content", () => {
    expect(p("a", b("b"))).toEqual({
      type: "paragraph",
      content: [{ type: "text", text: "a" }, b("b")],
    });
    expect(p()).toEqual({ type: "paragraph" });
    expect(h(2, "T")).toEqual({
      type: "heading",
      attrs: { level: 2 },
      content: [{ type: "text", text: "T" }],
    });
  });

  it("builds lists, tasks and list blocks", () => {
    expect(types(ul("a", p("b")))).toEqual([
      "bulletList",
      "listItem",
      "paragraph",
      "text",
      "listItem",
      "paragraph",
      "text",
    ]);
    expect(ol("a").attrs).toEqual({ start: 1 });
    expect(tasks([true, "done"]).content?.[0]?.attrs).toEqual({ checked: true });
    expect(listBlock("steps", "one").content?.[0]?.type).toBe("orderedList");
    expect(listBlock("facts", "k: v").content?.[0]?.type).toBe("bulletList");
  });

  it("builds containers", () => {
    expect(callout("warn", p("x")).attrs).toEqual({ kind: "warn" });
    expect(details("More", p("x")).attrs).toEqual({ summary: "More" });
    expect(columns([p("a")], [p("b")]).content?.map((column) => column.type)).toEqual([
      "column",
      "column",
    ]);
    expect(tabs(["One", p("x")]).content?.[0]).toMatchObject({
      type: "tab",
      attrs: { title: "One" },
    });
    expect(quote(p("x")).type).toBe("blockquote");
  });

  it("builds leaf blocks", () => {
    expect(hr()).toEqual({ type: "horizontalRule" });
    expect(toc()).toEqual({ type: "toc" });
    expect(fence("chart", "values: 1").attrs).toEqual({ lang: "chart", args: "" });
    expect(fence("table", "a", "sortable").attrs).toEqual({ lang: "table", args: "sortable" });
    expect(codeBlock("ts", "let x").attrs).toEqual({ language: "ts" });
    expect(mathBlock("x").content).toEqual([{ type: "text", text: "x" }]);
    expect(linkCard("https://a.example", "A").attrs).toEqual({
      url: "https://a.example",
      title: "A",
    });
    expect(embed("https://a.example").attrs).toEqual({ url: "https://a.example" });
    expect(image("a.png", "Alt").attrs).toMatchObject({ src: "a.png", alt: "Alt" });
  });

  it("builds tables with a header row", () => {
    const built = table(["A", "B"], ["1", "2"]);
    expect(built.content?.map((row) => row.content?.map((cell) => cell.type))).toEqual([
      ["tableHeader", "tableHeader"],
      ["tableCell", "tableCell"],
    ]);
  });

  it("only uses node types the schema defines", () => {
    const document_ = body(
      h(1, "T"),
      ul("a"),
      ol("b"),
      tasks([false, "c"]),
      callout("info", p()),
      details("d", p()),
      columns([p()]),
      tabs(["t", p()]),
      listBlock("cards", "x"),
      toc(),
      fence("chart", "x"),
      codeBlock("ts", "x"),
      mathBlock("x"),
      table(["a"], ["b"]),
      linkCard("u", "t"),
      embed("u"),
      image("s", "a"),
      quote(p()),
      hr(),
      p(mention("a"), wiki("w"), issue("I-1"), date("d"), math("m")),
    );
    const unknown = types(document_).filter((type) => type !== "text" && !NODE_NAMES.has(type));
    expect(unknown).toEqual([]);
  });
});

describe("withIds", () => {
  it("gives block nodes stable, unique ids and leaves others alone", () => {
    const document_ = body(p("a"), callout("info", p("b")), p(mention("sam")));
    const first = withIds("page", document_);
    const again = withIds("page", document_);
    expect(again).toEqual(first);
    const ids = [first.content?.[0], first.content?.[1]?.content?.[0], first.content?.[2]].map(
      (node): unknown => node?.attrs?.["blockId"],
    );
    expect(ids.every((id) => isBlockId(id))).toBe(true);
    expect(new Set(ids).size).toBe(3);
    expect(first.attrs).toBeUndefined();
    expect(first.content?.[2]?.content?.[0]).toEqual(mention("sam"));
  });

  it("depends on the page id and keeps other attributes", () => {
    const document_ = body(h(2, "x"));
    const heading = withIds("one", document_).content?.[0];
    expect(heading?.attrs?.["level"]).toBe(2);
    expect(heading?.attrs?.["blockId"]).not.toBe(
      withIds("two", document_).content?.[0]?.attrs?.["blockId"],
    );
  });

  it("tolerates nodes without a type", () => {
    expect(withIds("page", {})).toEqual({});
  });
});
