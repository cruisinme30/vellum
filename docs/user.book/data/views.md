---
id: 01M63CFE6A6EN0Y8BD7X9JSH88
title: Views, filters and embeds
vellum: 1
---

# Views, filters and embeds

A view is a saved way of looking at a collection. Every view shows the
same rows; changing a value in one view changes it everywhere.

## View types

::: cards

- ▦ **Table** Rows and columns; resize, reorder and hide columns.
- 🗂 **Board** Cards in columns grouped by a select, person or status property; drag to change the value.
- 🖼 **Gallery** Cards with the row's cover image.
- 📅 **Calendar** Rows on their date property.
- 📈 **Timeline** Rows as bars between a start and end date.
- 📋 **List** A compact list of titles.

:::

## Filters, sorts and groups {#filters}

Select **Edit view** above the view, or **New view** to add one. The
panel sets the layout, the filters, the sorts, the property to group by,
the dates for calendars and timelines, the hidden properties and how
many rows to show.

Write one filter per line as a property, a test and a value, such as
`Status is Done`, `Estimate > 3` or `Owner is empty`; rows must match
every line. Write one sort per line, adding `desc` for largest or latest
first. Hidden properties stay in the rows; they are only left out of the
view.

**Save** stores the view for everyone who can edit the collection.
**Delete view** removes it after a second press.

## Embed a view in a page {#embed}

Type `/collection` in any page and pick a collection and view, or write
a [collection fence](../writing/interactive.md#collection-views). The
embedded view is live: people can edit rows in place if they may edit
the collection.

## Live charts {#charts}

A [chart fence](../writing/charts.md#live-charts-from-a-collection)
with `source=collection:` plots a view's rows and redraws when they
change.
