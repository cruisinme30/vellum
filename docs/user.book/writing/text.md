---
id: 01M63CFE67J4PKTBWV4SEKZVFD
title: Text and links
vellum: 1
---

# Text and links

## Emphasis {#emphasis}

**bold** *italic* ~~strike~~ `code`

````markdown {corpus="inline/emphasis"}
**bold** *italic* ~~strike~~ `code`
````

Use `⌘B`, `⌘I` and `⌘E` in the editor, or type the markers.

## Highlight, subscript and superscript {#highlight}

==highlight== H~2~O x^2^

````markdown {corpus="inline/highlight-sub-sup"}
==highlight== H~2~O x^2^
````

## Links {#links}

[Runbook](https://vellum.northstar.dev/d/runbook) and <https://northstar.dev>

````markdown {corpus="inline/links"}
[Runbook](https://vellum.northstar.dev/d/runbook) and <https://northstar.dev>
````

Paste a URL onto selected text to make it a link.

## Wikilinks {#wikilinks}

Link to another page by its title with double brackets. Add `|` and a
label to change the text.

See [[Gateway architecture]] or [[Gateway architecture|the gateway page]].

````markdown {corpus="inline/wikilinks"}
See [[Gateway architecture]] or [[Gateway architecture|the gateway page]].
````

When you save, the editor stores the link as `vellum://doc/<id>`, so
renaming or moving the target page never breaks it. Type `[[` in the
editor to pick a page from a list.

## Mentions {#mentions}

Ask @priya and @team/engineering to review.

````markdown {corpus="inline/mentions"}
Ask @priya and @team/engineering to review.
````

Each person you mention gets an inbox item with a link to the block.

## Dates {#dates}

Ship on 2026-11-24, review @today, demo @next-friday.

````markdown {corpus="inline/dates"}
Ship on 2026-11-24, review @today, demo @next-friday.
````

Dates show as chips in your locale's format. Relative dates such as
`@today` are resolved when you save, so the page stays correct later.

## Issue keys {#issue-keys}

The replay engine (ENG-142) is in review.

````markdown {corpus="inline/issue-key"}
The replay engine (ENG-142) is in review.
````

Any issue key in a page becomes a live chip that shows the issue's
title, status and assignee, and updates when the issue changes.

## Line breaks and paragraphs {#line-breaks}

A blank line starts a new paragraph. To break a line inside a paragraph,
end it with a backslash.

Line one\
Line two

````markdown {corpus="inline/hard-break"}
Line one\
Line two
````

## Escaping {#escaping}

A backslash makes any special character literal.

\*not italic\* and \[[not a link]]

````markdown {corpus="inline/escapes"}
\*not italic\* and \[[not a link]]
````

## Inline math {#inline-math}

The F1 score is $\frac{2PR}{P+R}$.

````markdown {corpus="inline/math"}
The F1 score is $\frac{2PR}{P+R}$.
````

See [math blocks](structure.md#math-blocks) for display equations.
