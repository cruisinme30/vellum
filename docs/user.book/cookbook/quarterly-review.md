---
id: 01M63CFE6GJS1RY6ECPE79TGSK
title: A quarterly review book
vellum: 1
---

# A quarterly review book

**You will build** a quarterly business review as a book: one chapter
per team, each assembled from live collection views and charts, then
frozen as a snapshot and exported as one PDF.

## Steps

1. Create a book "Q4 2026 review" in the leadership shared drive.
2. Add chapters: Summary, Engineering, Operations, Design, Next quarter.
3. In each team chapter, embed that team's goals collection, filtered to the
   quarter.
4. Add a live chart that reads the goals collection, so the numbers are never
   typed by hand.
5. In the Summary, link to each chapter with `[[` so the links show "§2
   Engineering" and renumber if you reorder.
6. When the quarter closes, choose **⋯ → Take snapshot** and name it "Q4 2026
   final". Live blocks in the snapshot keep the values they had.
7. Export the snapshot with **Whole book** as a PDF for the board pack.

## A team chapter

`````markdown {corpus="page/cookbook-quarterly-chapter" live=false}
---
id: 01JB9F5G7H9J1K3M5N7P9Q1R3S
title: Engineering
vellum: 1
---

::: stats

- 11 of 14 · Goals met · ▲ 3
- 34 · Average velocity · ▲ 4 pts
- 99.97% · Availability · ▬ 0

:::

```chart {source=collection:01JB9G6H8J0K2M4N6P8Q0R2S4T x=Quarter y=Points agg=sum}
title: Points shipped by quarter
type: bar
```

```collection
id: 01JB9G6H8J0K2M4N6P8Q0R2S4T
view: By status
filter: Quarter == "2026-Q4"
```
`````
