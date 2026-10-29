---
id: 01M64VT4GZD6NQYK0CHP3X5KWC
title: Video storyboard
vellum: 1
---

# Video storyboard

The ten help videos, one per common task, each under two minutes. Every
scene's caption is also its WebVTT cue. Tours that exist are in
`web/e2e/tours`; record them with `pnpm run tours`. Each one uses the
Northstar demo workspace, so names, dates and numbers match the guide.

## How a tour is written {#tours}

```ts
await tour.scene("Type a slash, then a few letters to filter the blocks.", async () => {
  await page.keyboard.type("/callout", { delay: 120 });
  await expect(page.getByRole("listbox", { name: "Blocks" })).toBeVisible();
});
```

A scene runs its steps, holds the screen long enough to read the caption
and records the cue. Keep captions under fifteen words, in the guide's
voice, and keep each tour under two minutes.

## The ten videos {#ten}

| # | Task | Chapter or recipe | Tour |
|---|------|-------------------|------|
| 1 | Create and share a page | Your first ten minutes | `01-create-and-share-a-page` |
| 2 | Use the slash menu | Your first ten minutes | `02-use-the-slash-menu` |
| 3 | Add a chart and a diagram | Charts and data tables | To write |
| 4 | Create a collection and a board view | Views, filters and embeds | `04-create-a-board-view` |
| 5 | Create a book and reorder its sections | Books | To write |
| 6 | Export to PDF and Word | Exporting a chapter as PDF | To write |
| 7 | Create and triage issues | Running a cycle | To write |
| 8 | Quick-add tasks and subscribe to the calendar feed | A personal task list | To write |
| 9 | Connect Claude over MCP and review a draft | Release notes from an MCP prompt | To write |
| 10 | Restore a page from version history | The editor | To write |

## Scenes for the tours still to write {#scenes}

3. **A chart and a diagram.** Open "Gateway architecture". Insert
   `/chart`: "Paste numbers and pick a chart type." Insert `/mermaid`:
   "Describe the request path in text; Vellum draws it." Switch to dark
   theme: "Both follow the theme."
5. **A book.** Open **Books → Northstar handbook**. "A book is a folder
   of pages with a table of contents." Drag "On-call" above
   "Onboarding": "Reorder by dragging; numbering follows." Open
   **History**: "Every change to the contents is kept."
6. **Export.** On "PRD: Policy simulator" choose **⋯ → Export → PDF**:
   "Charts and diagrams stay sharp." Repeat with Word: "Headings,
   tables and comments carry over."
7. **Issues.** **Issues → ENG → New issue** "Gateway times out under
   load": "Issues live in teams." Open **Triage**: "Accept, decline or
   mark a duplicate." Assign Priya and set the cycle: "It shows in her
   My Work."
8. **Tasks.** Press `Q` and type "Send launch checklist to legal
   tomorrow 3pm p1": "Dates and priorities are typed, not picked." Open
   **My Work → ⋯ → Calendar feed**: "Subscribe once; dated tasks show
   in your calendar."
9. **Claude.** In **Admin → AI**, copy the Claude Desktop snippet:
   "One snippet connects Claude." Ask Claude for release notes from the
   ENG cycle: "Claude writes a draft, never the page." Open the draft
   and accept it: "You review every change."
10. **History.** Open "Platform roadmap 2027", choose **History**, pick
    yesterday's version: "Every version is kept." Choose **Restore**:
    "Restoring makes a new version; nothing is lost."

Write each tour once its screens are in the web app. A tour whose
screens need a server records against `vellum serve` loaded with the
Northstar workspace instead of the demo build.
