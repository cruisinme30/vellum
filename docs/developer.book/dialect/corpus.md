---
id: 01M63CFE6597AEJEXQFBJ1VETP
title: The golden corpus
vellum: 1
---

# The golden corpus

The golden corpus is a folder of Vellum Markdown files with the output
expected from each stage of the pipeline. The user guide's examples are
part of it: they are extracted from the guide, so the guide and the
tests can never drift apart.

## Layout {#layout}

```text
corpus/
  format/                       # cases written for the parser
  guide/                        # generated from the user guide
    container/info.md           # the input
    container/info.ast.json     # parse output and diagnostics
    container/info.html         # rendered HTML
    container/info.canonical.md # only when canonical form differs
```

## Checks {#checks}

For every input, the test suite checks that:

- parsing gives the expected tree and diagnostics
- serializing gives the input, or its canonical form
- the result parses back to the same tree
- rendering gives the expected HTML, PDF text and Word structure
- charts and diagrams match their pixel fixtures within 0.1%

Run `VELLUM_BLESS=1 cargo test -p vellum-format --test it corpus::` to
regenerate expected files after a deliberate change, and review the
diff.

## Examples from the guide {#from-the-guide}

[Maintaining the help books](../contributing/help-books.md) explains how
examples are marked in the guide and how `cargo xtask help` extracts
them.
