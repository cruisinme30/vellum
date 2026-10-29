---
id: 01M63CFE6ACQRBNH8HC5HE1WAT
title: Numbering and cross-references
vellum: 1
---

# Numbering and cross-references

## Numbering

Vellum numbers sections from their place in the book: 1, 1.1, 1.1.1.
Numbers are computed, never typed, so moving a section renumbers it and
every reference to it. **Book settings → Numbering** chooses decimal
(1.2.3), outline (1, a, i) or none.

## Cross-references {#cross-references}

Type `[[` and pick a section to link to it. In a book, a link to another
section shows its number and title, such as "§2.1 Services", and keeps
both up to date when the section moves or is renamed. Exports turn the
link into a page reference such as "see §2.1 Services on page 14".

The link is stored by ID, so it survives any rename:

See [§](vellum://doc/01JB6A1C3E5G7J9L2N4Q6S8U0W) for the service list.

````markdown {corpus="inline/cross-reference"}
See [§](vellum://doc/01JB6A1C3E5G7J9L2N4Q6S8U0W) for the service list.
````

## Headings and sections {#convert}

Select a heading and choose **Make section** to move it and the text
below it into a new child section. Choose **Merge into parent** on a
section to turn it back into a heading. The links into it keep working
either way.

## Moving sections between books {#move}

Drag a section, with its children, to another book in the sidebar, or
use **⋯ → Move to**. Links to the moved sections are rewritten. **Copy
to** makes new pages with new IDs, so the copy and the original evolve
separately.
