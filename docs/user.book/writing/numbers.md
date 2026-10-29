---
id: 01M63CFE6872ZE4R5D3BBVQ222
title: Numbers and facts
vellum: 1
---

# Numbers and facts

These containers turn short lists into tiles, key-value panels, progress
bars, numbered steps and cards. Each list item is one entry.

## Stats

Each item is `value · label · delta`. A delta starting with ▲ shows as
up, ▼ as down and ▬ as flat.

::: stats

- 94% · Injections caught · ▲ 6 pts
- 38 ms · p95 latency · ▼ 12 ms
- 12 · Open incidents · ▬ 0

:::

````markdown {corpus="container/stats"}
::: stats

- 94% · Injections caught · ▲ 6 pts
- 38 ms · p95 latency · ▼ 12 ms
- 12 · Open incidents · ▬ 0

:::
````

## Facts

Each item is `key: value`.

::: facts

- Owner: Priya Shah
- Status: In review
- Target: 2026-11-24

:::

````markdown {corpus="container/facts"}
::: facts

- Owner: Priya Shah
- Status: In review
- Target: 2026-11-24

:::
````

## Progress

Each item is `label · percent`.

::: progress

- Replay engine · 100%
- Diff report · 70%
- PDF report · 15%

:::

````markdown {corpus="container/progress"}
::: progress

- Replay engine · 100%
- Diff report · 70%
- PDF report · 15%

:::
````

## Steps

A numbered list whose bold text becomes each step's title.

::: steps

1. **Confirm the dashboard** is green
2. **Announce** in the deploys channel
3. **Tag** the release

:::

````markdown {corpus="container/steps"}
::: steps

1. **Confirm the dashboard** is green
2. **Announce** in the deploys channel
3. **Tag** the release

:::
````

## Cards

Each item is an emoji, a bold title and a description.

::: cards

- 🛰 **Edge listener** Terminates TLS.
- ⚖️ **Policy engine** Evaluates rules.
- 🗄 **Audit writer** Records every decision.

:::

````markdown {corpus="container/cards"}
::: cards

- 🛰 **Edge listener** Terminates TLS.
- ⚖️ **Policy engine** Evaluates rules.
- 🗄 **Audit writer** Records every decision.

:::
````
