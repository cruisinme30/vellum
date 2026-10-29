// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor, JSONContent } from "@tiptap/core";
import { afterEach, describe, expect, it } from "vitest";

import { PAGE } from "../../data/mock/ids";
import { NORTHSTAR_BODIES } from "../../data/mock/northstar-pages";
import {
  body,
  callout,
  codeBlock,
  columns,
  details,
  embed,
  fence,
  image,
  linkCard,
  mathBlock,
  p,
  table,
  tabs,
  toc,
} from "../content";
import { createTestEditor, topLevelTypes, typeText } from "../test-editor";

let editor: Editor | undefined;
const open = (content: JSONContent | string) => {
  editor = createTestEditor({ content });
  return editor;
};
afterEach(() => {
  editor?.destroy();
  editor = undefined;
});

/** Parses the editor's HTML into a detached element for querying. */
function htmlOf(target: Editor): HTMLElement {
  const container = document.createElement("div");
  container.innerHTML = target.getHTML();
  return container;
}

const prd = NORTHSTAR_BODIES[PAGE.prd];

describe("block nodes", () => {
  it("loads the PRD page, which uses every block, without dropping any", () => {
    if (!prd) {
      throw new Error("the PRD page is missing");
    }
    const target = open(prd);
    expect(target.getJSON()).toMatchObject(prd);
  });

  it("round-trips the PRD page through its own HTML", () => {
    if (!prd) {
      throw new Error("the PRD page is missing");
    }
    const target = open(prd);
    const loaded = target.getJSON();
    target.commands.setContent(target.getHTML());
    expect(target.getJSON()).toEqual(loaded);
  });

  it("renders schema-built containers as divs with their data-type", () => {
    const container = htmlOf(
      open(
        body(callout("warn", p("careful")), columns([p("a")], [p("b")]), details("More", p("x"))),
      ),
    );
    const note = container.querySelector<HTMLElement>('[data-type="callout"]');
    expect(note?.getAttribute("role")).toBe("note");
    expect(note?.dataset["kind"]).toBe("warn");
    expect(note?.className).toBe("vl-block vl-callout");
    expect(container.querySelectorAll('[data-type="column"]')).toHaveLength(2);
    expect(container.querySelector<HTMLElement>('[data-type="details"]')?.dataset["summary"]).toBe(
      "More",
    );
  });

  it("renders tabs, toc and atom blocks with their attributes", () => {
    const target = open(
      body(
        toc(),
        tabs(["One", p("1")]),
        linkCard("https://example.com", "Example"),
        embed("https://youtu.be/abcdefgh"),
        image("/a.png", "A"),
      ),
    );
    const container = htmlOf(target);
    expect(container.querySelector('nav[data-type="toc"]')).not.toBeNull();
    target.commands.setContent(container.innerHTML);
    expect(topLevelTypes(target)).toEqual(["toc", "tabs", "linkCard", "embed", "image"]);
    expect(container.querySelector<HTMLElement>('[data-type="tab"]')?.dataset["title"]).toBe("One");
    expect(container.querySelector<HTMLElement>('[data-type="linkCard"]')?.dataset["url"]).toBe(
      "https://example.com",
    );
    expect(container.querySelector('[data-type="embed"]')?.childNodes).toHaveLength(0);
    expect(container.querySelector<HTMLElement>('[data-type="image"]')?.dataset["align"]).toBe(
      "center",
    );
  });

  it("writes a data fence as pre > code and reads it back from the code element", () => {
    const target = open(body(fence("chart", "values: [1, 2]", "wide")));
    const container = htmlOf(target);
    const fenceElement = container.querySelector<HTMLElement>('[data-type="dataFence"]');
    expect(fenceElement?.querySelector("pre > code")?.textContent).toBe("values: [1, 2]");
    expect(fenceElement?.dataset["args"]).toBe("wide");
    target.commands.setContent(container.innerHTML);
    expect(target.getJSON().content[0]).toMatchObject({
      type: "dataFence",
      attrs: { lang: "chart", args: "wide" },
      content: [{ type: "text", text: "values: [1, 2]" }],
    });
  });

  it("writes a code block's language only when it has one", () => {
    const container = htmlOf(open(body(codeBlock("rust", "fn main() {}"), codeBlock("", "plain"))));
    const [rust, plain] = container.querySelectorAll("pre");
    expect(rust?.dataset["language"]).toBe("rust");
    expect(rust?.className).toBe("vl-code");
    expect(plain?.hasAttribute("data-language")).toBe(false);
  });

  it("writes no language for a code block whose language is unset", () => {
    const target = open(body(codeBlock("rust", "x")));
    target.commands.command(({ tr }) => {
      // eslint-disable-next-line unicorn/no-null -- ProseMirror's unset attribute
      tr.setNodeAttribute(0, "language", null);
      return true;
    });
    expect(htmlOf(target).querySelector("pre")?.hasAttribute("data-language")).toBe(false);
  });

  it("reads a code block's language from a language- class", () => {
    const target = open(
      '<pre><code class="language-ts">let a = 1;</code></pre><pre><code>x</code></pre>',
    );
    const [typed, untyped] = target.getJSON().content;
    expect(typed?.attrs?.["language"]).toBe("ts");
    expect(untyped?.attrs?.["language"]).toBe("");
  });

  it("indents code with Tab", () => {
    const target = open(body(codeBlock("", "x")));
    target.commands.setTextSelection(1);
    target.commands.keyboardShortcut("Tab");
    expect(target.state.doc.firstChild?.textContent).toMatch(/^\s+x$/);
  });

  it("writes display math as pre > code and keeps its whitespace", () => {
    const target = open(body(mathBlock("a  +\n b")));
    const container = htmlOf(target);
    expect(container.querySelector('pre[data-type="mathBlock"] > code')?.textContent).toBe(
      "a  +\n b",
    );
    target.commands.setContent(container.innerHTML);
    expect(topLevelTypes(target)).toEqual(["mathBlock"]);
    expect(target.state.doc.firstChild?.textContent).toBe("a  +\n b");
  });

  it("turns $$ and a space at the start of a line into a math block", () => {
    const target = open(body(p()));
    target.commands.setTextSelection(1);
    typeText(target, "$$ ");
    expect(topLevelTypes(target)).toEqual(["mathBlock"]);
  });

  it("gives table cells an alignment and holds one paragraph per cell", () => {
    const target = open(body(table(["A", "B"], ["1", "2"])));
    const container = htmlOf(target);
    expect(container.querySelector("table")?.className).toBe("vl-table");
    expect(container.querySelector("th")?.dataset["align"]).toBe("none");
    target.commands.setContent('<table><tr><td data-align="right"><p>x</p></td></tr></table>');
    const cell = target.state.doc.firstChild?.firstChild?.firstChild;
    expect(cell?.type.name).toBe("tableCell");
    expect(cell?.attrs["align"]).toBe("right");
  });
});
