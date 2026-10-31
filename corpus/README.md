<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Golden corpus

Every block type as Vellum Markdown with its expected AST, HTML, PDF text
and Word structure, plus pixel fixtures for diagrams and charts. The
round-trip test (parse, serialize, parse) must be identical. The corpus
arrives with `vellum-format`.

Export goldens live in `export/`, mirroring `format/` and `guide/`: for
every page, `.pdf.txt` is the print PDF's text page by page and
`.docx.txt` the Word file's structure (paragraph styles, list levels and
table rows). Rewrite them with `VELLUM_BLESS=1 cargo test -p
vellum-export --test it golden::` after an intended change, and review the
diff.
