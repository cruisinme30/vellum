// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { getSchema, type Editor, type JSONContent } from "@tiptap/core";
import type { Attrs, MarkType, NodeType } from "@tiptap/pm/model";
import { describe, expect, it } from "vitest";

import { vellumExtensions } from "./extensions";
import { MARKS, NODES } from "./schema/blocks.gen";
import {
  BLOCK_ID_TYPES,
  nodeSpec,
  SCHEMA_NAMES,
  tiptapAttributes,
} from "./schema/spec";
import type { AttributeData } from "./schema/types";
import {
  createTestEditor,
  english,
  resolveGateway,
  typeText,
} from "./test-editor";

const byName = (left: string, right: string) => left.localeCompare(right);

const schema = getSchema(
  vellumExtensions({ t: english, resolvePage: resolveGateway, history: false }),
);

/** The default of every attribute ProseMirror knows for a type. */
function defaults(type: NodeType | MarkType): Record<string, unknown> {
  const attributes = (type.spec.attrs ?? {}) as Record<
    string,
    { default?: unknown }
  >;
  return Object.fromEntries(
    Object.entries(attributes).map(([key, value]) => [key, value.default]),
  );
}

/** Stock Tiptap extensions mark an empty string attribute as unset. */
function sameDefault(actual: unknown, expected: AttributeData): boolean {
  if (expected.type === "string" && expected.default === "") {
    return actual === "" || actual === null;
  }
  return actual === expected.default;
}

describe("schema parity with blocks.gen.ts", () => {
  it.each(NODES.map((node) => [node.name, node] as const))(
    "has node %s with its content, group and attribute defaults",
    (name, spec) => {
      const type = schema.nodes[name];
      expect(type, `node ${name}`).toBeDefined();
      if (!type) {
        return;
      }
      expect(type.spec.content ?? "").toBe(spec.content);
      expect(type.spec.group ?? "").toBe(spec.group);
      const actual = defaults(type);
      for (const [key, attribute] of Object.entries(
        spec.attrs as Record<string, AttributeData>,
      )) {
        expect(key in actual, `${name}.${key}`).toBe(true);
        expect(
          sameDefault(actual[key], attribute),
          `${name}.${key} default`,
        ).toBe(true);
      }
      if ("inline" in spec) {
        expect(type.isInline).toBe(spec.inline);
      }
      if ("atom" in spec) {
        expect(type.isAtom).toBe(spec.atom);
      }
    },
  );

  it.each(MARKS.map((mark) => [mark.name, mark] as const))(
    "has mark %s with its attribute defaults and exclusions",
    (name, spec) => {
      const type = schema.marks[name];
      expect(type, `mark ${name}`).toBeDefined();
      if (!type) {
        return;
      }
      const actual = defaults(type);
      for (const [key, attribute] of Object.entries(
        spec.attrs as Record<string, AttributeData>,
      )) {
        expect(
          sameDefault(actual[key], attribute),
          `${name}.${key} default`,
        ).toBe(true);
      }
      if ("excludes" in spec) {
        expect(type.spec.excludes).toBe(spec.excludes);
      }
    },
  );

  it("gives exactly the schema's block types a blockId attribute", () => {
    const withIds = Object.values(schema.nodes)
      .filter((type) => "blockId" in (type.spec.attrs ?? {}))
      .map((type) => type.name);
    expect(withIds.toSorted(byName)).toEqual(
      [...BLOCK_ID_TYPES].toSorted(byName),
    );
  });

  it("lists every node and mark name", () => {
    expect(SCHEMA_NAMES.nodes).toContain("dataFence");
    expect(SCHEMA_NAMES.marks).toContain("highlight");
    expect(Object.keys(schema.nodes).toSorted(byName)).toEqual(
      [...SCHEMA_NAMES.nodes].toSorted(byName),
    );
    expect(Object.keys(schema.marks).toSorted(byName)).toEqual(
      [...SCHEMA_NAMES.marks].toSorted(byName),
    );
  });

  it("keeps subscript and superscript apart", () => {
    const editor = createTestEditor({ content: "<p>x2</p>" });
    editor.commands.setTextSelection({ from: 2, to: 3 });
    editor.commands.toggleSubscript();
    editor.commands.toggleSuperscript();
    expect(editor.getHTML()).toContain("x<sup>2</sup></p>");
    expect(editor.getHTML()).not.toContain("<sub>");
    editor.destroy();
  });
});

const task = (attributes?: string): JSONContent => ({
  type: "taskList",
  content: [
    {
      type: "taskItem",
      attrs: {
        checked: false,
        ...(attributes === undefined ? {} : { attributes }),
      },
      content: [
        { type: "paragraph", content: [{ type: "text", text: "Ship" }] },
      ],
    },
  ],
});
const itemAttributes = (editor: Editor) =>
  editor.state.doc.firstChild?.firstChild?.attrs;

describe("task item attributes", () => {
  it(
    "keeps a task's {…} attribute span through getJSON and an HTML round " +
      "trip",
    () => {
      const editor = createTestEditor({
        content: { type: "doc", content: [task("{due=2026-11-24}")] },
      });
      expect(itemAttributes(editor)?.["attributes"]).toBe("{due=2026-11-24}");
      const html = editor.getHTML();
      expect(html).toContain('data-attributes="{due=2026-11-24}"');
      editor.commands.setContent(html);
      expect(itemAttributes(editor)?.["attributes"]).toBe("{due=2026-11-24}");
      editor.destroy();
    },
  );

  it(
    "writes no data-attributes for a task without a span, and reads it back " +
      "empty",
    () => {
      const editor = createTestEditor({
        content: { type: "doc", content: [task()] },
      });
      expect(itemAttributes(editor)?.["attributes"]).toBe("");
      const html = editor.getHTML();
      expect(html).not.toContain("data-attributes");
      editor.commands.setContent(html);
      expect(itemAttributes(editor)?.["attributes"]).toBe("");
      editor.commands.command(({ tr }) => {
        // eslint-disable-next-line unicorn/no-null -- ProseMirror's unset attribute
        tr.setNodeAttribute(1, "attributes", null);
        return true;
      });
      expect(editor.getHTML()).not.toContain("data-attributes");
      editor.destroy();
    },
  );
});

describe("links", () => {
  it("keeps http, https, mailto and relative links and drops other schemes", () => {
    const editor = createTestEditor({
      content:
        '<p><a href="https://a.example">a</a> <a ' +
        'href="mailto:x@a.example">m</a> ' +
        '<a href="/p/1">r</a> <a href="javascript:alert(1)">j</a> <a ' +
        'href="ftp://a.example">f</a></p>',
    });
    const hrefs: unknown[] = [];
    editor.state.doc.descendants((node) => {
      for (const mark of node.marks) {
        hrefs.push(mark.attrs["href"]);
      }
    });
    expect(hrefs).toEqual(["https://a.example", "mailto:x@a.example", "/p/1"]);
    expect(editor.getHTML()).toContain('rel="noopener noreferrer"');
    expect(editor.getHTML()).not.toContain("target=");
    editor.destroy();
  });

  it("turns a typed web address into a link", () => {
    const editor = createTestEditor({ content: "<p></p>" });
    editor.commands.setTextSelection(1);
    typeText(editor, "see https://vellum.example ");
    expect(editor.getHTML()).toContain('href="https://vellum.example"');
    editor.destroy();
  });

  it.each([
    ["<p></p>", "Type / to insert a block"],
    ["<h1></h1>", ""],
  ])("gives the empty block %s the placeholder %j", (content, text) => {
    const editor = createTestEditor({ content });
    expect(
      editor.view.dom.querySelector<HTMLElement>("[data-placeholder]")?.dataset[
        "placeholder"
      ],
    ).toBe(text);
    editor.destroy();
  });
});

describe("nodeSpec", () => {
  it("returns the generated data for a node", () => {
    expect(nodeSpec("callout").attrs).toHaveProperty("kind");
  });

  it("throws for a name outside the schema", () => {
    expect(() => nodeSpec("nope" as "doc")).toThrow("no node nope");
  });
});

function element(attributes: Record<string, string>) {
  const div = document.createElement("div");
  for (const [key, value] of Object.entries(attributes)) {
    div.setAttribute(key, value);
  }
  return div;
}

/**
 * Parses one attribute of a node from an element with the given HTML
 * attributes.
 */
function parse(
  name: Parameters<typeof nodeSpec>[0],
  key: string,
  html: Record<string, string>,
): unknown {
  return tiptapAttributes(nodeSpec(name))[key]?.parseHTML?.(element(html));
}

describe("tiptapAttributes", () => {
  it("reads every attribute type from data-* and falls back to defaults", () => {
    expect(parse("heading", "level", { "data-level": "3" })).toBe(3);
    expect(parse("heading", "level", { "data-level": "x" })).toBe(1);
    expect(parse("taskItem", "checked", { "data-checked": "true" })).toBe(true);
    expect(parse("taskItem", "checked", { "data-checked": "no" })).toBe(false);
    expect(parse("callout", "kind", { "data-kind": "warn" })).toBe("warn");
    expect(parse("callout", "kind", { "data-kind": "shout" })).toBe("info");
    expect(parse("image", "src", { "data-src": "/a.png" })).toBe("/a.png");
    expect(parse("image", "src", {})).toBe("");
  });

  it("skips transient attributes, which are never saved", () => {
    const spec = {
      ...nodeSpec("details"),
      attrs: {
        summary: { type: "string", default: "" },
        open: { type: "boolean", default: false, transient: true },
      },
    } satisfies ReturnType<typeof nodeSpec>;
    expect(Object.keys(tiptapAttributes(spec))).toEqual(["summary"]);
  });

  it("writes camelCase attributes as kebab-case data attributes", () => {
    const html = tiptapAttributes(nodeSpec("dataFence"))["lang"]?.renderHTML?.({
      lang: "chart",
    } satisfies Attrs);
    expect(html).toEqual({ "data-lang": "chart" });
  });
});
