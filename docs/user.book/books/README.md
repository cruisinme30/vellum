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

1. Select **New → Book** and give it a title, such as "Engineering handbook".
2. Add sections with **+** in the book's left navigation.
3. Drag sections to reorder them, or onto another section to nest them.
4. Select a section to write it like any other page.

To turn an existing long page into a book, open **⋯ → Convert to book**.
Vellum makes a section of every `##` heading. **⋯ → Merge into one page**
turns a section and its children back into headings.

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

A section inherits the book's access. In **Section settings** the book
owner can narrow access to a section, name its owners and name the
reviewers who must approve changes to it. Readers who cannot see a
section see the rest of the book, numbered without it.

## History and snapshots {#history}

Each section keeps its own history, and the book keeps a history of its
order and structure. **⋯ → Take snapshot** names the whole book as it
stands, such as "v1.0"; compare two snapshots to see every change
across all sections.
