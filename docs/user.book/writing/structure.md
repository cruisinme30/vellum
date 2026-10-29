---
id: 01M63CFE67KJ8DNXFCVQYT4F27
title: Structure, lists and tables
vellum: 1
---

# Structure, lists and tables

## Headings

Use `#` for the page title and `##` to `####` for sections. Headings
feed the table of contents, the outline panel and, in books, the
numbering. If both the front matter and an H1 give a title, the front
matter wins.

````markdown {corpus="block/headings" live=false}
# Page title
## Section
### Subsection
#### Minor heading
````

A heading takes a custom anchor as an attribute, so a link to it
survives a change of wording:

````markdown {corpus="block/heading-anchor" live=false}
## Rollback {#rollback}
````

## Dividers and quotes

---

````markdown {corpus="block/thematic-break"}
---
````

> A policy you cannot simulate
> is a policy you cannot trust.

````markdown {corpus="block/blockquote"}
> A policy you cannot simulate
> is a policy you cannot trust.
````

## Table of contents

A `toc` container lists the headings that follow it on the page.

::: toc
:::

````markdown {corpus="container/toc"}
::: toc
:::
````

## Lists

- Bullet
- Another
  - Nested (two spaces)
    - Deeper

````markdown {corpus="block/bullet-list"}
- Bullet
- Another
  - Nested (two spaces)
    - Deeper
````

1. First
2. Second
   1. Sub-step

````markdown {corpus="block/ordered-list"}
1. First
2. Second
   1. Sub-step
````

## Tasks

- [ ] Announce in the deploys channel
- [x] Tag the release

````markdown {corpus="block/task-list"}
- [ ] Announce in the deploys channel
- [x] Tag the release
````

A task with a person and a date is an **assignment**. It appears in
that person's [My Work](../my-work/README.md) and calendar feed, and
ticking it anywhere ticks it here.

- [ ] Review the PRD @maya 2026-11-03

````markdown {corpus="block/task-assigned"}
- [ ] Review the PRD @maya 2026-11-03
````

## Code

```bash
helm rollback gateway 1 -n prod
kubectl rollout status deploy/gateway
```

`````markdown {corpus="fence/code"}
```bash
helm rollback gateway 1 -n prod
kubectl rollout status deploy/gateway
```
`````

Vellum highlights about 180 languages. Attributes add a title, line
numbers and highlighted lines:

```rust {title="limits.rs" lines=1 hl="3-4"}
pub fn allowed(rate: u32, burst: u32) -> bool {
    let budget = rate.saturating_mul(2);
    let used = burst;
    used <= budget
}
```

`````markdown {corpus="fence/code-attributes"}
```rust {title="limits.rs" lines=1 hl="3-4"}
pub fn allowed(rate: u32, burst: u32) -> bool {
    let budget = rate.saturating_mul(2);
    let used = burst;
    used <= budget
}
```
`````

## Math blocks

Write display math between `$$` lines in LaTeX syntax. Vellum renders it
as MathML, so it scales and reads aloud in screen readers.

$$
F_1 = \frac{2PR}{P+R}
$$

````markdown {corpus="block/math"}
$$
F_1 = \frac{2PR}{P+R}
$$
````

A `math` fence does the same and suits longer derivations:

```math
\sigma = \sqrt{\frac{1}{N}\sum_{i=1}^{N}(x_i - \mu)^2}
```

`````markdown {corpus="fence/math"}
```math
\sigma = \sqrt{\frac{1}{N}\sum_{i=1}^{N}(x_i - \mu)^2}
```
`````

## Tables

| # | Requirement | Status |
| --- | --- | --- |
| R1 | Replay 7 days | Done |
| R2 | Diff report | In review |
| R3 | PDF report | Blocked |

````markdown {corpus="table/status"}
| # | Requirement | Status |
| --- | --- | --- |
| R1 | Replay 7 days | Done |
| R2 | Diff report | In review |
| R3 | PDF report | Blocked |
````

Words such as Done, Blocked, In review, P0, Yes and No become coloured
chips automatically. Type `|` at the start of a line to start a table in
the editor.

### Comparison matrix

A table whose cells are ✓, ✗ and ◐ (or yes, no and part) renders as a
comparison matrix.

| | Option A | Option B |
| --- | --- | --- |
| Self-hosted | ✓ | ✗ |
| Free SSO | ✓ | ◐ |

````markdown {corpus="table/compare"}
| | Option A | Option B |
| --- | --- | --- |
| Self-hosted | ✓ | ✗ |
| Free SSO | ✓ | ◐ |
````

For tables you can sort and draw bars in, see [data
tables](charts.md#data-tables).
