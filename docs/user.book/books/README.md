---
id: 01M63CFE6ANMXKF0XQHKFM57BG
title: Books
vellum: 1
---

# Books

A **book** is a document made of documents: a handbook, a specification,
a course or this guide. Each section is its own page, with its own
history, comments and permissions, and the book puts them in order,
numbers them and exports them as one document.

## Create a book {#create}

1. Select **New → Book**, or **New book** on the Books screen. Give it a
   title, such as "Engineering handbook", choose the drive it goes in
   (My Drive or a shared drive), and select **Create book**. The book
   opens, empty.
2. Select **Add the first section**, then add more with **Add section
   after** or **Add subsection** above the section you are reading.
3. Drag a section onto the top or bottom edge of another to put it
   before or after, or onto its middle to nest it. From the keyboard,
   press Alt+Shift with an arrow: up and down reorder, right nests under
   the section above, left moves out.
4. Select a section to write it like any other page.

**Headings to sections** makes a child section of every heading in the
section; **Sections to headings** folds its children back in, and links
to them now point at the headings. **Duplicate** copies a section with
everything under it, and **Move to another book** moves or copies it
into another book (a move keeps links working).

## A book is a folder

```text
handbook.book/
  book.yaml
  SUMMARY.md
  README.md
  onboarding/README.md
  onboarding/first-week.md
  architecture/README.md
  architecture/services.md
```

`book.yaml` holds the book's settings and the identity of each section.
`SUMMARY.md` holds the order and nesting in the same syntax as mdBook
and GitBook, so the folder builds as a website with an unmodified
mdBook.

````markdown {corpus="page/book-summary" live=false}
# Summary

[Introduction](README.md)

# Part I · Joining

- [Onboarding](onboarding/README.md)
  - [Your first week](onboarding/first-week.md)
  - [Security training]()

# Part II · How it works

- [Architecture](architecture/README.md)
  - [Services](architecture/services.md)

---

[Glossary](glossary.md)
````

::: facts

- `[Title](file.md)` before the first list: a prefix chapter, not numbered
- `# Part`: a part title that groups the chapters below it
- `- [Title](file.md)`: a numbered chapter; indent to nest
- `[Title]()`: a draft chapter that has no page yet
- `---`: a separator
- links after the last list: suffix chapters, not numbered

:::

Books nest three levels deep by default. The book owner can raise the
limit to six in **Book settings**.

## Section permissions {#permissions}

A section inherits the book's access. In **Section access** a book manager can
narrow access to a section (as `user:` or `group:` IDs), name its owners and
name the reviewers who must approve changes to it. Readers who cannot see a
section see the rest of the book, numbered without it.

## History and snapshots {#history}

Each section keeps its own history, and **Outline history** keeps every
change to the book's order and structure; **Restore this order** puts
the sections back as they were. In **Snapshots**, **Take snapshot**
names the whole book as it stands, such as "v1.0"; **Compare** shows
every change between two snapshots, or between one and the book now.
