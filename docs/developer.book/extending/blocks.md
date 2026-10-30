---
id: 01M63CFE66BW169QP9T1MJWF2B
title: Writing a block
vellum: 1
---

# Writing a block

A block is a fence or container name, a schema entry and a renderer.

::: steps

1. **Declare it** in `schema/` with its name, attributes and children.
2. **Generate** the Rust and ProseMirror types.
3. **Render it** in `vellum-render`: HTML through the sanitiser, or SVG for
   drawings.
4. **Export it** in `vellum-export`: a vector image or a table for PDF and Word.
5. **Edit it** in the web app: a Tiptap node view and a slash menu entry.
6. **Document it** in the user guide, with the block and its source marked for
   the corpus.

:::

The build fails until step 6 is done: `cargo xtask help` checks that every known
block, listed in `xtask/src/help/coverage.rs`, has an example in the guide. Add
your block to that list.

## Community blocks {#plugins}

Community blocks are WebAssembly renderers. They receive the fence's
text and attributes and return SVG or sanitised HTML. They have no
network access and no access to the page beyond their own box.

## Rules for every block {#rules}

- Readable without Vellum: on GitHub the block must make sense as text or code.
- Accessible: a text alternative for every drawing, keyboard access to every
  control.
- Themed: colours come from theme tokens, never hard-coded.
- Never fatal: bad input renders the source with a diagnostic.
