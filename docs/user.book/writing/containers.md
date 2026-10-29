---
id: 01M63CFE68RASNTPVVBTRXBA6Z
title: Callouts and containers
vellum: 1
---

# Callouts and containers

A container starts with `:::` and a name, and ends with `:::`. Anything
inside is ordinary Vellum Markdown, including other containers. On
GitHub and in plain Markdown viewers a container shows as its text.

## Callouts

Six callouts set a tone: info, warn, good, crit, tip and note.

::: info
The gateway is the single entry point.
:::

````markdown {corpus="container/info-short"}
::: info
The gateway is the single entry point.
:::
````

::: warn
A rewrite is not approved.
:::

````markdown {corpus="container/warn"}
::: warn
A rewrite is not approved.
:::
````

::: good
Ship the simulator before the Q4 review.
:::

````markdown {corpus="container/good"}
::: good
Ship the simulator before the Q4 review.
:::
````

::: crit
Never roll back the audit writer separately.
:::

````markdown {corpus="container/crit"}
::: crit
Never roll back the audit writer separately.
:::
````

::: tip
Press / to insert any block.
:::

````markdown {corpus="container/tip"}
::: tip
Press / to insert any block.
:::
````

::: note
Replays use production traffic with personal data removed.
:::

````markdown {corpus="container/note"}
::: note
Replays use production traffic with personal data removed.
:::
````

Vellum also reads GitHub-style callouts, so Markdown written for GitHub
renders the same way:

> [!note]
> Callouts written for GitHub work too.

````markdown {corpus="container/github-callout"}
> [!note]
> Callouts written for GitHub work too.
````

## Toggles

A `details` container is collapsed until you select its title.

::: details Why the audit log is written first
So a blocked request still leaves a trace.
:::

````markdown {corpus="container/details"}
::: details Why the audit log is written first
So a blocked request still leaves a trace.
:::
````

## Columns

Two to four columns, separated by `---` lines. On a phone the columns
stack.

::: columns
**Problem**
Policy changes ship blind.

---

**Outcome**
Every change ships with a report.
:::

````markdown {corpus="container/columns"}
::: columns
**Problem**
Policy changes ship blind.

---

**Outcome**
Every change ships with a report.
:::
````

## Tabs

Each `##` heading inside a `tabs` container starts a tab. Only the
selected tab shows; exports print every tab one after another.

::: tabs

## Overview

The engine evaluates each request.

## Limits

Rule sets are capped at 2,000.
:::

````markdown {corpus="container/tabs"}
::: tabs

## Overview

The engine evaluates each request.

## Limits

Rule sets are capped at 2,000.
:::
````

## Nesting containers

Containers nest: give the outer container one more colon than the one
inside it.

:::: columns
::: info
Containers nest: add a colon per level.
:::

---

Second column
::::

````markdown {corpus="container/nested"}
:::: columns
::: info
Containers nest: add a colon per level.
:::

---

Second column
::::
````

## Unknown containers

A container Vellum does not know renders as a plain box with its
contents, so a page written for a newer version of Vellum still reads
well. Vellum never refuses to show a page because of a block it does not
understand.

::: sidebar
Contents of a container this version does not know.
:::

````markdown {corpus="container/unknown"}
::: sidebar
Contents of a container this version does not know.
:::
````
