---
id: 01M63CFE6GHMBHHQ9VRGTC42YZ
title: A private research wiki
vellum: 1
---

# A private research wiki

**You will build** a research wiki in an end-to-end encrypted space,
where pages are encrypted on members' devices and the server stores
only ciphertext.

::: warn
In an end-to-end encrypted space, the server cannot read your pages.
That means server-side search, previews, public links and MCP over the
network do not work there. Search and MCP run on the desktop app
instead. If every member loses their keys, nobody can recover the pages.
:::

## Steps

1. Install the desktop app; end-to-end encrypted spaces need it.
2. Select **New → Space**, call it "Pricing research" and turn on **End-to-end encryption**.
3. Add members. Each member's device holds their own key; Vellum encrypts every page to all members.
4. Create a book "Interviews" with one section per interview, and a collection "Findings" with a relation to the interviews.
5. Write a summary page that embeds the findings board.
6. To remove a member, remove them from the space; Vellum re-encrypts new versions without their key.

## A finding

`````markdown {corpus="page/cookbook-research-finding" live=false}
---
id: 01JB9J8K0M2N4P6Q8R0S2T4V6W
title: Buyers compare on audit, not price
status: verified
owner: maya
tags: [pricing, research]
vellum: 1
---

::: facts

- Interviews: 7 of 9
- Confidence: high
- Segment: regulated enterprises

:::

> We only shortlisted gateways that could show us every blocked
> request for a year.

## Evidence

- [[Interview 03: bank, platform team]]
- [[Interview 07: insurer, security team]]
`````
