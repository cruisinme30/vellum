// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A small Markdown reader for book sections: headings, paragraphs, lists,
 * quotes, code blocks and rules, with code, bold, italic and links inline.
 * It builds React elements and never HTML, so a section can't inject markup;
 * anything it doesn't know reads as plain text.
 */

import { Fragment, type ReactNode } from "react";

type Block =
  | { readonly kind: "heading"; readonly level: number; readonly text: string }
  | { readonly kind: "paragraph" | "quote"; readonly text: string }
  | {
      readonly kind: "list";
      readonly ordered: boolean;
      readonly items: readonly string[];
    }
  | { readonly kind: "code"; readonly text: string }
  | { readonly kind: "rule" };

const HEADING = /^(#{1,6}) (.*)$/;
const BULLET = /^ *[-*+] +(\S.*)$/;
const NUMBERED = /^ *\d+[.)] +(\S.*)$/;
const RULE = /^(?:-{3,}|\*{3,}|_{3,})$/;
const FENCE = /^\s*(```|~~~)/;

/** Collects lines while `test` holds, starting at `start`. */
function run(
  lines: readonly string[],
  start: number,
  test: (line: string) => boolean,
): number {
  let end = start;
  while (end < lines.length && test(lines[end] ?? "")) end += 1;
  return end;
}

/** One block starting at `index`, and where the next one starts. */
function blockAt(lines: readonly string[], index: number): [Block, number] {
  const line = lines[index] ?? "";
  const fence = FENCE.exec(line);
  if (fence) {
    const close = run(
      lines,
      index + 1,
      (each) => !each.trimStart().startsWith(fence[1] ?? ""),
    );
    return [
      { kind: "code", text: lines.slice(index + 1, close).join("\n") },
      close + 1,
    ];
  }
  const heading = HEADING.exec(line);
  if (heading) {
    // A closing run of hashes is decoration: "## Title ##".
    const text = (heading[2] ?? "").replace(/ #+$/, "").trim();
    return [
      { kind: "heading", level: heading[1]?.length ?? 1, text },
      index + 1,
    ];
  }
  if (RULE.test(line.replaceAll(" ", ""))) return [{ kind: "rule" }, index + 1];
  for (const [pattern, ordered] of [
    [BULLET, false],
    [NUMBERED, true],
  ] as const) {
    if (pattern.test(line)) {
      const end = run(lines, index, (each) => pattern.test(each));
      const items = lines
        .slice(index, end)
        .map((each) => pattern.exec(each)?.[1] ?? "");
      return [{ kind: "list", ordered, items }, end];
    }
  }
  if (line.startsWith(">")) {
    const end = run(lines, index, (each) => each.startsWith(">"));
    const text = lines
      .slice(index, end)
      .map((each) => each.replace(/^>\s?/, ""))
      .join(" ");
    return [{ kind: "quote", text }, end];
  }
  const end = run(
    lines,
    index,
    (each) =>
      each.trim() !== "" &&
      !HEADING.test(each) &&
      !FENCE.test(each) &&
      !BULLET.test(each) &&
      !NUMBERED.test(each) &&
      !each.startsWith(">"),
  );
  return [
    { kind: "paragraph", text: lines.slice(index, end).join(" ") },
    Math.max(end, index + 1),
  ];
}

/** Splits Markdown into blocks. */
function parseBlocks(markdown: string): Block[] {
  const lines = markdown.replaceAll("\r\n", "\n").split("\n");
  const blocks: Block[] = [];
  let index = 0;
  while (index < lines.length) {
    if ((lines[index] ?? "").trim() === "") {
      index += 1;
      continue;
    }
    const [block, next] = blockAt(lines, index);
    blocks.push(block);
    index = next;
  }
  return blocks;
}

/** Inline marks, each with the node it makes; the earliest match wins. */
const INLINE: readonly {
  readonly pattern: RegExp;
  readonly node: (parts: readonly string[], key: string) => ReactNode;
}[] = [
  {
    pattern: /`([^`]+)`/,
    node: ([text], key) => <code key={key}>{text}</code>,
  },
  {
    pattern: /\*\*([^*]+)\*\*/,
    node: ([text = ""], key) => <strong key={key}>{inline(text)}</strong>,
  },
  {
    pattern: /\*([^*]+)\*/,
    node: ([text = ""], key) => <em key={key}>{inline(text)}</em>,
  },
  {
    pattern: /_([^_]+)_/,
    node: ([text = ""], key) => <em key={key}>{inline(text)}</em>,
  },
  {
    pattern: /\[([^\]]{1,300})\]\(([^)\s]{1,2000})\)/,
    node: ([label = "", href = ""], key) => link(label, href, key),
  },
];

/** Only web and mail links open; anything else reads as text. */
const safeHref = (href: string) =>
  /^(https?:|mailto:|\/|#)/i.test(href) ? href : undefined;

function link(label: string, href: string, key: string): ReactNode {
  const safe = safeHref(href);
  return safe === undefined ? (
    <Fragment key={key}>{inline(label)}</Fragment>
  ) : (
    <a key={key} href={safe} rel="noreferrer">
      {inline(label)}
    </a>
  );
}

/** The earliest inline mark in `text`. */
function nextMark(text: string) {
  let best: { at: number; length: number; node: ReactNode } | undefined;
  for (const rule of INLINE) {
    const match = rule.pattern.exec(text);
    if (match === null || (best !== undefined && match.index >= best.at))
      continue;
    const key = String(match.index);
    best = {
      at: match.index,
      length: match[0].length,
      node: rule.node(match.slice(1), key),
    };
  }
  return best;
}

/** Inline Markdown as React nodes. */
function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let rest = text;
  let offset = 0;
  for (let mark = nextMark(rest); mark !== undefined; mark = nextMark(rest)) {
    if (mark.at > 0) out.push(rest.slice(0, mark.at));
    out.push(<Fragment key={offset + mark.at}>{mark.node}</Fragment>);
    offset += mark.at + mark.length;
    rest = rest.slice(mark.at + mark.length);
  }
  if (rest !== "") out.push(rest);
  return out;
}

function renderBlock(block: Block, key: number): ReactNode {
  switch (block.kind) {
    case "heading": {
      // The section title is the page's h1; its headings start at h2.
      const Tag = `h${String(Math.min(block.level + 1, 6))}` as "h2";
      return <Tag key={key}>{inline(block.text)}</Tag>;
    }
    case "paragraph": {
      return <p key={key}>{inline(block.text)}</p>;
    }
    case "quote": {
      return <blockquote key={key}>{inline(block.text)}</blockquote>;
    }
    case "code": {
      return (
        <pre key={key}>
          <code>{block.text}</code>
        </pre>
      );
    }
    case "rule": {
      return <hr key={key} />;
    }
    case "list": {
      const items = block.items.map((item, index) => (
        <li key={index}>{inline(item)}</li>
      ));
      return block.ordered ? (
        <ol key={key}>{items}</ol>
      ) : (
        <ul key={key}>{items}</ul>
      );
    }
  }
}

/**
 * A section's Markdown, read-only, without a first heading that repeats
 * `title`.
 */
export function MarkdownReader({
  markdown,
  title,
}: {
  readonly markdown: string;
  readonly title?: string;
}) {
  const blocks = parseBlocks(markdown);
  const [first] = blocks;
  const shown =
    first?.kind === "heading" && first.text.trim() === title?.trim()
      ? blocks.slice(1)
      : blocks;
  return (
    <div className="vl-markdown">
      {shown.map((block, index) => renderBlock(block, index))}
    </div>
  );
}
