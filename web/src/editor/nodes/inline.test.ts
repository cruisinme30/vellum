// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Editor, Extension, type JSONContent } from "@tiptap/core";
import { StarterKit } from "@tiptap/starter-kit";
import { afterEach, describe, expect, it } from "vitest";

import { body, date, issue, math, mention, p, wiki } from "../content";
import { createTestEditor, typeText } from "../test-editor";

import { atomRule, findWikiLink, formatDate, IssueKey } from "./inline";

// A resolver that never finds a page.
// eslint-disable-next-line unicorn/no-useless-undefined
const noPage = () => undefined;

let editor: Editor | undefined;
const open = (content: JSONContent = body(p())) => {
  editor = createTestEditor({ content });
  editor.commands.setTextSelection(1);
  return editor;
};
afterEach(() => {
  editor?.destroy();
  editor = undefined;
});

/** The inline content of the first paragraph, without block attributes. */
function inlineOf(target: Editor): JSONContent[] {
  return target.getJSON().content[0]?.content ?? [];
}

const typed = (text: string) => {
  const target = open();
  typeText(target, text);
  return inlineOf(target);
};

describe("inline input rules", () => {
  it("turns @handle and a space into a mention chip", () => {
    expect(typed("hi @priya ")).toEqual([
      { type: "text", text: "hi " },
      mention("priya"),
      { type: "text", text: " " },
    ]);
  });

  it("accepts group handles with slashes", () => {
    expect(typed("@eng/platform ")).toEqual([
      mention("eng/platform"),
      { type: "text", text: " " },
    ]);
  });

  it("leaves an e-mail address as text", () => {
    expect(typed("mail a@b.c ")).toEqual([
      { type: "text", text: "mail a@b.c " },
    ]);
  });

  it("turns @today into today's date chip, not a mention", () => {
    expect(typed("due @today ")).toEqual([
      { type: "text", text: "due " },
      date("today"),
      { type: "text", text: " " },
    ]);
  });

  it("turns an ISO date into a date chip", () => {
    expect(typed("ship 2026-11-24 ")).toEqual([
      { type: "text", text: "ship " },
      date("2026-11-24"),
      { type: "text", text: " " },
    ]);
  });

  it("ignores impossible dates", () => {
    expect(typed("2026-13-01 ")).toEqual([
      { type: "text", text: "2026-13-01 " },
    ]);
  });

  it("turns an issue key into an issue chip", () => {
    expect(typed("see ENG-142 ")).toEqual([
      { type: "text", text: "see " },
      issue("ENG-142"),
      { type: "text", text: " " },
    ]);
  });

  it("does not treat ENG-0 as an issue key", () => {
    expect(typed("ENG-0 ")).toEqual([{ type: "text", text: "ENG-0 " }]);
  });

  it("turns [[Target|label]] into a wiki link with no trailing text", () => {
    expect(typed("go [[Gateway|the gateway]]")).toEqual([
      { type: "text", text: "go " },
      wiki("Gateway", "the gateway"),
    ]);
  });

  it("turns [[Target]] into a wiki link with an empty label", () => {
    expect(typed("[[Rate limits]]")).toEqual([wiki("Rate limits")]);
  });

  it("turns $tex$ into inline math", () => {
    expect(typed(String.raw`area $\pi r^2$`)).toEqual([
      { type: "text", text: "area " },
      math(String.raw`\pi r^2`),
    ]);
  });

  it("does not take a lone dollar amount for math", () => {
    expect(typed("costs $ 5 $")).toEqual([
      { type: "text", text: "costs $ 5 $" },
    ]);
  });
});

describe("atomRule", () => {
  it("uses the whole match as the token when the pattern has no groups", () => {
    const shortcut = Extension.create({
      name: "shortcut",
      addInputRules() {
        const type = () => {
          const found = this.editor.schema.nodes["issueKey"];
          if (!found) {
            throw new Error("no issueKey type");
          }
          return found;
        };
        return [atomRule(/#1$/, type, () => ({ key: "ENG-1" }))];
      },
    });
    const target = new Editor({
      extensions: [StarterKit, IssueKey, shortcut],
      content: "<p></p>",
    });
    editor = target;
    target.commands.setTextSelection(1);
    typeText(target, "#1");
    expect(inlineOf(target)).toEqual([issue("ENG-1")]);
  });
});

describe("findWikiLink", () => {
  it("finds the last [[…]] at the end of the text", () => {
    expect(findWikiLink("see [[Gateway | docs ]]")).toEqual({
      index: 4,
      text: "[[Gateway | docs ]]",
      data: { token: "[[Gateway | docs ]]", target: "Gateway", label: "docs" },
    });
  });

  it.each([
    ["no closing brackets", "[[Gateway"],
    ["no opening brackets", "Gateway]]"],
    ["an empty target", "[[ ]]"],
    ["an empty pair", "[[]]"],
    ["a stray bracket in the target", "[[a]b]]"],
    ["a stray bracket in the label", "[[a|b]c]]"],
    ["an empty target before a label", "[[ |label]]"],
  ])("rejects %s", (_name, text) => {
    expect(findWikiLink(text)).toBeNull();
  });
});

describe("formatDate", () => {
  it("keeps today as Today", () => {
    expect(formatDate("today")).toBe("Today");
  });

  it("formats an ISO date in UTC for the locale", () => {
    expect(formatDate("2026-11-24")).toBe("Nov 24, 2026");
    expect(formatDate("2026-11-24", "de")).toBe("24. Nov. 2026");
  });

  it("passes anything else through", () => {
    expect(formatDate("next week")).toBe("next week");
  });
});

describe("inline chips in HTML and text", () => {
  const content = body(
    p(
      mention("dev"),
      " ",
      wiki("Gateway"),
      " ",
      wiki("Gateway", "gw"),
      " ",
      wiki("Nowhere"),
      " ",
      issue("ENG-7"),
      " ",
      date("2026-11-24"),
      " ",
      date("today"),
      " ",
      math("x^2"),
    ),
  );

  it("renders each chip with its data-type and label", () => {
    const html = open(content).getHTML();
    const container = document.createElement("div");
    container.innerHTML = html;
    const chips = [
      ...container.querySelectorAll<HTMLElement>("[data-type]"),
    ].map((element) => [
      element.tagName.toLowerCase(),
      element.dataset["type"],
      element.textContent,
    ]);
    expect(chips).toEqual([
      ["span", "mention", "@dev"],
      ["a", "wikiLink", "📄 Gateway"],
      ["a", "wikiLink", "📄 gw"],
      ["a", "wikiLink", "📄 Nowhere"],
      ["span", "issueKey", "ENG-7"],
      ["time", "date", "Nov 24, 2026"],
      ["time", "date", "Today"],
      ["span", "mathInline", "x^2"],
    ]);
  });

  it("links a resolved wiki target to its page and marks a miss", () => {
    const container = document.createElement("div");
    container.innerHTML = open(content).getHTML();
    const [hit, labelled, miss] = container.querySelectorAll("a");
    expect(hit?.getAttribute("href")).toBe("/p/page-gateway");
    expect(hit?.className).toBe("vl-wikilink");
    expect(labelled?.getAttribute("href")).toBe("/p/page-gateway");
    expect(miss?.getAttribute("href")).toBe("#");
    expect(miss?.className).toBe("vl-wikilink vl-missing");
  });

  it("gives a dated chip a datetime but not Today", () => {
    const container = document.createElement("div");
    container.innerHTML = open(content).getHTML();
    const times = [...container.querySelectorAll("time")].map((time) =>
      time.hasAttribute("datetime") ? time.getAttribute("datetime") : "none",
    );
    expect(times).toEqual(["2026-11-24", "none"]);
  });

  it("writes chips back as their Markdown source in plain text", () => {
    expect(open(content).getText()).toBe(
      "@dev [[Gateway]] [[Gateway]] [[Nowhere]] ENG-7 2026-11-24 @today $x^2$",
    );
  });

  it("parses chips back from HTML", () => {
    const target = open(content);
    const html = target.getHTML();
    target.commands.setContent(html);
    expect(inlineOf(target)).toEqual(content.content?.[0]?.content);
  });

  it("links every wiki target to # without a resolver", () => {
    editor = createTestEditor({ content, resolvePage: noPage });
    expect(editor.getHTML()).not.toContain("/p/");
  });
});
