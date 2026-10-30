// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  InputRule,
  mergeAttributes,
  Node,
  type InputRuleFinder,
  type InputRuleMatch,
} from "@tiptap/core";
import type { NodeType } from "@tiptap/pm/model";

import type { NodeName } from "../schema/blocks.gen";
import { nodeSpec, tiptapAttributes } from "../schema/spec";
import { mediaUrl } from "../urls";

/**
 * An input rule that turns the `token` group of a match into an inline atom
 * and keeps whatever follows it (normally the space that triggered it).
 */
export function atomRule(
  find: InputRuleFinder,
  type: () => NodeType,
  attributes: (
    groups: Record<string, string | undefined>,
  ) => Record<string, string>,
) {
  return new InputRule({
    find,
    handler: ({ state, range, match }) => {
      const groups = (match.groups ?? match.data ?? {}) as Record<
        string,
        string | undefined
      >;
      const token = groups["token"] ?? match[0];
      const offset = match[0].lastIndexOf(token);
      const tail = match[0].slice(offset + token.length);
      const node = type().create(attributes(groups));
      const nodes = tail === "" ? [node] : [node, state.schema.text(tail)];
      state.tr.replaceWith(range.from + offset, range.to, nodes);
    },
  });
}

/** Formats an ISO date for a chip; `today` stays "Today". */
export function formatDate(value: string, locale = "en"): string {
  if (value === "today") {
    return "Today";
  }
  const parsed = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!parsed) {
    return value;
  }
  const [, year, month, day] = parsed.map(Number) as [
    number,
    number,
    number,
    number,
  ];
  return new Date(Date.UTC(year, month - 1, day)).toLocaleDateString(locale, {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}

const inlineBase = (name: NodeName) => {
  const spec = nodeSpec(name);
  return {
    name: spec.name,
    group: "inline",
    inline: true,
    atom: true,
    selectable: true,
    addAttributes: () => tiptapAttributes(spec),
    // Above the default 50: a wiki link is an `<a href>`, which the link mark
    // would take.
    parseHTML: () => [{ tag: `[data-type="${spec.name}"]`, priority: 60 }],
  };
};

/**
 * Finds `[[Target]]` or `[[Target|label]]` just typed, without a backtracking
 * regex.
 */
export function findWikiLink(text: string): InputRuleMatch | null {
  const start = text.lastIndexOf("[[");
  if (!text.endsWith("]]") || start === -1 || start > text.length - 5) {
    // eslint-disable-next-line unicorn/no-null -- Tiptap's "no match"
    return null;
  }
  const inner = text.slice(start + 2, -2);
  const bar = inner.indexOf("|");
  const target = (bar === -1 ? inner : inner.slice(0, bar)).trim();
  const label = bar === -1 ? "" : inner.slice(bar + 1).trim();
  if (target === "" || target.includes("]") || label.includes("]")) {
    // eslint-disable-next-line unicorn/no-null -- Tiptap's "no match"
    return null;
  }
  const token = text.slice(start);
  return { index: start, text: token, data: { token, target, label } };
}

/** Resolves a wikilink target to a page, for its label and link. */
export type ResolvePage = (
  target: string,
) => { id: string; title: string } | undefined;

const handle = String.raw`[a-z][a-z0-9_/-]*`;

/** `@handle`: a person or group. */
export const Mention = Node.create({
  ...inlineBase("mention"),
  renderHTML: ({ node, HTMLAttributes }) => [
    "span",
    mergeAttributes(HTMLAttributes, {
      "data-type": "mention",
      class: "vl-chip vl-mention",
    }),
    `@${String(node.attrs["handle"])}`,
  ],
  renderText: ({ node }) => `@${String(node.attrs["handle"])}`,
  addInputRules() {
    return [
      atomRule(
        new RegExp(
          String.raw`(?:^|\s)(?<token>@(?!today\b)(?<handle>${handle}))\s$`,
        ),
        () => this.type,
        (g) => ({ handle: g["handle"] ?? "" }),
      ),
    ];
  },
});

/** `[[Title]]` or `[[Title|label]]`: a link to another page. */
export const WikiLink = Node.create<{ resolve?: ResolvePage }>({
  ...inlineBase("wikiLink"),
  addOptions: () => ({}),
  renderHTML({ node, HTMLAttributes }) {
    const target = String(node.attrs["target"]);
    const label = String(node.attrs["label"]);
    const page = this.options.resolve?.(target);
    return [
      "a",
      mergeAttributes(HTMLAttributes, {
        "data-type": "wikiLink",
        class: page ? "vl-wikilink" : "vl-wikilink vl-missing",
        href: page ? `/p/${page.id}` : "#",
      }),
      `📄 ${label === "" ? (page?.title ?? target) : label}`,
    ];
  },
  renderText: ({ node }) => `[[${String(node.attrs["target"])}]]`,
  addInputRules() {
    return [
      atomRule(
        findWikiLink,
        () => this.type,
        (g) => ({
          target: g["target"]?.trim() ?? "",
          label: g["label"]?.trim() ?? "",
        }),
      ),
    ];
  },
});

/** `ENG-142`: an issue key, shown as a chip (live status arrives with Work). */
export const IssueKey = Node.create({
  ...inlineBase("issueKey"),
  renderHTML: ({ node, HTMLAttributes }) => [
    "span",
    mergeAttributes(HTMLAttributes, {
      "data-type": "issueKey",
      class: "vl-chip vl-issue",
    }),
    String(node.attrs["key"]),
  ],
  renderText: ({ node }) => String(node.attrs["key"]),
  addInputRules() {
    return [
      atomRule(
        /(?:^|\s)(?<token>(?<key>[A-Z][A-Z0-9]{1,9}-[1-9]\d*))\s$/,
        () => this.type,
        (g) => ({ key: g["key"] ?? "" }),
      ),
    ];
  },
});

/** `2026-11-24` or `@today`: a date chip. */
export const DateChip = Node.create({
  ...inlineBase("date"),
  renderHTML: ({ node, HTMLAttributes }) => {
    const value = String(node.attrs["date"]);
    return [
      "time",
      mergeAttributes(HTMLAttributes, {
        "data-type": "date",
        class: "vl-chip vl-date",
        ...(value === "today" ? {} : { datetime: value }),
      }),
      formatDate(value),
    ];
  },
  renderText: ({ node }) => {
    const value = String(node.attrs["date"]);
    return value === "today" ? "@today" : value;
  },
  addInputRules() {
    return [
      atomRule(
        /(?:^|\s)(?<token>(?<date>\d{4}-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12]\d|3[01])))\s$/,
        () => this.type,
        (g) => ({ date: g["date"] ?? "" }),
      ),
      atomRule(
        /(?:^|\s)(?<token>@today)\s$/,
        () => this.type,
        () => ({ date: "today" }),
      ),
    ];
  },
});

/** `$x^2$`: inline TeX. */
export const MathInline = Node.create({
  ...inlineBase("mathInline"),
  renderHTML: ({ node, HTMLAttributes }) => [
    "span",
    mergeAttributes(HTMLAttributes, {
      "data-type": "mathInline",
      class: "vl-math-inline",
    }),
    String(node.attrs["literal"]),
  ],
  renderText: ({ node }) => `$${String(node.attrs["literal"])}$`,
  addInputRules() {
    return [
      atomRule(
        /(?:^|\s)(?<token>\$(?<tex>[^\s$](?:[^$]*[^\s$])?)\$)$/,
        () => this.type,
        (g) => ({ literal: g["tex"] ?? "" }),
      ),
    ];
  },
});

/** `![alt](src)` inside a line of text or a link. */
export const InlineImage = Node.create({
  ...inlineBase("inlineImage"),
  draggable: true,
  parseHTML: () => [{ tag: 'img[data-type="inlineImage"]', priority: 60 }],
  renderHTML: ({ node, HTMLAttributes }) => {
    const source = mediaUrl(String(node.attrs["src"]));
    const title = String(node.attrs["title"]);
    return [
      "img",
      mergeAttributes(HTMLAttributes, {
        "data-type": "inlineImage",
        class: "vl-inline-image",
        alt: String(node.attrs["alt"]),
        ...(source === undefined ? {} : { src: source }),
        ...(title === "" ? {} : { title }),
      }),
    ];
  },
  renderText: ({ node }) =>
    `![${String(node.attrs["alt"])}](${String(node.attrs["src"])})`,
});
