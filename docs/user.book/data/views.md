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

Select **Filter** to add conditions. Each condition is a property, an
operator (contains, is, is not, before, after, is empty) and a value.
Combine conditions with `and` or `or`.

**Sort** orders rows by one or more properties. **Group** splits a table
or list into sections by a property. **Properties** hides columns
without deleting them.

Changes to a view are yours until you select **Save for everyone**.

## Embed a view in a page {#embed}

Type `/collection` in any page and pick a collection and view, or write
a [collection fence](../writing/interactive.md#collection-views). The
embedded view is live: people can edit rows in place if they may edit
the collection.

## Live charts {#charts}

A [chart fence](../writing/charts.md#live-charts-from-a-collection)
with `source=collection:` plots a view's rows and redraws when they
change.
