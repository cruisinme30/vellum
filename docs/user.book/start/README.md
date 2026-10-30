---
id: 01M63CFE67X46N7KX44C7WV4HP
title: Your first ten minutes
vellum: 1
---

# Your first ten minutes

This chapter takes you from an empty workspace to a page you have
written, shared and exported. It assumes someone has already installed
Vellum and given you an account; if you are that someone, start with the
[admin guide](../../admin.book/deploy/README.md).

::: steps

1. **Sign in** with your organisation's single sign-on or the email and password
   your admin gave you.
2. **Create a page** with **New** in the navigation rail, or press `⌘K` and type
   "new page".
3. **Write** a title, then press `/` on an empty line to insert any block.
4. **Share** with the **Share** button: add people or groups, then pick view,
   comment or edit.
5. **Export** with **⋯ → Export**: PDF, Word, Markdown or HTML.

:::

```image
src: assets/editor.jpg
alt: The Vellum editor showing Northstar's "Gateway 2.0 launch plan" page, with the drive on the left and the Share and Export buttons at the top right
caption: Northstar's launch plan in the editor. The drive is on the left; Share and Export are at the top right.
```

## Create your first page {#first-page}

Select **New** in the navigation rail. Vellum creates an untitled page
in **My Drive** and puts the cursor in the title. Type a title such as
"Gateway launch plan" and press `Enter` to start the body.

The page is saved as you type. There is no save button: changes reach
everyone else on the page in well under a second, and Vellum writes a
version to disk after 30 seconds of quiet or when you leave the page.

## Insert blocks with the slash menu {#slash-menu}

On an empty line, type `/` to open the slash menu. Keep typing to
filter it: `/info` finds the info callout, `/chart` the chart, `/mer`
the Mermaid diagram. `Enter` inserts the highlighted block.

You can also type Markdown directly. The editor turns `#` into a
heading, `-` into a bullet, `[]` into a task, `>` into a quote, three
backticks into a code block and `:::` into a container as you type.

::: info
Every block you insert with the slash menu is stored as plain Vellum
Markdown. Open **⋯ → View source** to see it, or add `?format=md` to the
page URL.
:::

## Share the page {#share}

Select **Share** at the top right. Type a name, an email or a group,
pick a role and select **Share**.

::: facts

- Can view: read the page and its comments
- Can comment: view, plus comment and suggest
- Can edit: comment, plus change the page
- Full access: edit, plus share and move

:::

Pages inherit access from their folder. The [sharing
chapter](../drive/sharing.md) explains inheritance, public links and
how to restrict a single page.

## Export the page {#export}

Select **⋯ → Export** and pick a format. PDF and Word keep charts and
diagrams as vector images; live blocks such as collection views export
as their current state. The [rendering table](../writing/rendering.md)
lists how each block looks in each format.

## Where to go next

- Learn every block in [Writing in Vellum Markdown](../writing/README.md).
- Track work with [issues](../work/README.md) and your own tasks in [My
  Work](../my-work/README.md).
- Try a [cookbook recipe](../cookbook/README.md) from start to finish.
