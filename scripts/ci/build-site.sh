#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds the project website into a folder (default: _site):
#   - the landing page from site/,
#   - every help book docs/<name>.book/ with mdBook, at docs/<name>/,
#   - the architecture decision records docs/adr/ as one more book, at
#     docs/decisions/.
# Vellum Markdown is turned into plain CommonMark on the way: front matter
# is dropped and ::: container fences are unwrapped, so unmodified mdBook
# can build the books (ADR 0027, D111).
#
# Usage: scripts/ci/build-site.sh [output-dir]
# Environment: MDBOOK (default: mdbook), SITE_BASE (default: /vellum/), the
# path the site is served under, used only by mdBook's 404 page.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$(realpath -m "${1:-${root}/_site}")"
mdbook="${MDBOOK:-mdbook}"
base="${SITE_BASE:-/vellum/}"
repo="https://github.com/cruisinme30/vellum"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

# to_commonmark <file>: rewrites a Vellum Markdown page in place. Drops a
# leading YAML front matter block and the opening and closing lines of
# ::: containers (a details title becomes a bold line), rewrites relative
# links for the published layout (<name>.book/ becomes <name>/, README.md
# becomes index.md), and turns the
# front matter title into the page heading when the page has none. It
# never touches anything inside a code fence, where
# the help books show Vellum Markdown source on purpose.
to_commonmark() {
  awk '
    # Published books live at docs/<name>/ and mdBook writes README.md
    # chapters as index.html, so relative links follow both renames.
    function fix_target(t) {
      if (t ~ /^[A-Za-z][A-Za-z0-9+.-]*:/ || t ~ /^[\/#]/) return t
      gsub(/\.book\//, "/", t)
      if (t ~ /(^|\/)README\.md(#|$)/) sub(/README\.md/, "index.md", t)
      return t
    }
    function fix_links(s,   out, link) {
      out = ""
      while (match(s, /\]\([^) \t]+/)) {
        link = substr(s, RSTART + 2, RLENGTH - 2)
        out = out substr(s, 1, RSTART + 1) fix_target(link)
        s = substr(s, RSTART + RLENGTH)
      }
      out = out s
      if (match(out, /^ ? ? ?\[[^]]+\]:[ \t]*/) && RLENGTH < length(out)) {
        link = substr(out, RSTART + RLENGTH); sub(/[ \t].*$/, "", link)
        out = substr(out, 1, RLENGTH) fix_target(link) substr(out, RLENGTH + 1 + length(link))
      }
      return out
    }
    NR == 1 && $0 == "---" { front = 1; next }
    front {
      if ($0 == "---" || $0 == "...") front = 0
      else if ($0 ~ /^title:/) { heading = $0; sub(/^title:[ \t]*/, "", heading); gsub(/^["\047]|["\047]$/, "", heading) }
      next
    }
    heading != "" && $0 !~ /^[ \t]*$/ {
      if ($0 !~ /^# /) print "# " heading "\n"
      heading = ""
    }
    {
      if (fence == "") {
        if (match($0, /^ ? ? ?(```+|~~~+)/)) {
          run = substr($0, RSTART, RLENGTH); sub(/^ +/, "", run)
          fence = run; print; next
        }
        if ($0 ~ /^ ? ? ?:::+/) {
          title = $0; sub(/^ *:+ *details +/, "", title)
          if (title != $0 && title != "") print "**" title "**\n"
          next
        }
        $0 = fix_links($0)
      } else {
        line = $0; sub(/^ +/, "", line); sub(/[ \t]+$/, "", line)
        if (substr(line, 1, 1) == substr(fence, 1, 1) && line ~ /^(`+|~+)$/ && length(line) >= length(fence)) fence = ""
      }
      print
    }
  ' "$1" >"$1.tmp"
  mv "$1.tmp" "$1"
}

# yaml_title <file>: the top-level title: value of a YAML file or front
# matter, without quotes.
yaml_title() {
  sed -n 's/^title:[[:space:]]*//p' "$1" | head -n 1 | sed -e 's/^["'\'']//' -e 's/["'\'']$//'
}

html_escape() {
  sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g' <<<"$1"
}

# fix_html <dir>: patches mdBook 0.5.4's HTML where it fails WCAG 2.2 AA
# or breaks its own scripts (#249):
#   - the sidebar toggle is a <label> carrying button ARIA: give it the role;
#   - code blocks and tables that scroll sideways become focusable;
#   - task-list checkboxes get a name;
#   - mdBook's web fonts are dropped (system fonts, see book.css): they load
#     without font-display, so text appears late and the page shifts;
#   - headings named like a JavaScript module global (exports, module,
#     define, require) get a suffix: the browser exposes an element id as
#     a window property, and mdBook's search and copy libraries then take
#     the page for CommonJS and fail.
fix_html() {
  find "$1" -name '*.html' -exec sed -i -E \
    -e '/<link rel="stylesheet" href="fonts\/fonts-[0-9a-f]*\.css">/d' \
    -e 's/<label id="mdbook-sidebar-toggle" /<label id="mdbook-sidebar-toggle" role="button" /' \
    -e 's/<pre([^>]*)><code( |>)/<pre\1><code tabindex="0"\2/g' \
    -e 's/<div class="table-wrapper">/<div class="table-wrapper" tabindex="0">/g' \
    -e 's/<input disabled="" type="checkbox" checked="">/<input disabled="" type="checkbox" checked="" aria-label="Done">/g' \
    -e 's/<input disabled="" type="checkbox">/<input disabled="" type="checkbox" aria-label="To do">/g' \
    -e 's/id="(exports|module|define|require)"/id="\1-section"/g' \
    -e 's/href="([^"]*)#(exports|module|define|require)"/href="\1#\2-section"/g' {} +
}

# build_book <name> <title> <staged-src>: runs mdBook on a staged source.
build_book() {
  local name="$1" title="$2" src="$3" dir="${work}/$1" toml_title
  toml_title="${2//\\/\\\\}"
  toml_title="${toml_title//\"/\\\"}"
  find "${src}" -name '*.md' -print0 | while IFS= read -r -d '' page; do
    to_commonmark "${page}"
  done
  mkdir -p "${dir}/theme"
  cp "${root}/site/book.css" "${dir}/theme/vellum.css"
  cat >"${dir}/book.toml" <<EOF
[book]
title = "${toml_title}"
language = "en"
src = "$(realpath --relative-to="${dir}" "${src}")"

[output.html]
site-url = "${base}docs/${name}/"
git-repository-url = "${repo}"
default-theme = "light"
preferred-dark-theme = "navy"
additional-css = ["theme/vellum.css"]

# No "run" button: it calls play.rust-lang.org from every page with Rust
# code, which leaks readers to a third party and fails offline (#249).
[output.html.playground]
runnable = false
EOF
  "${mdbook}" build "${dir}" --dest-dir "${out}/docs/${name}"
  fix_html "${out}/docs/${name}"
  books+=("${name}"$'\t'"${title}")
}

rm -rf "${out}"
mkdir -p "${out}/docs"
cp "${root}/site/index.html" "${root}/site/style.css" "${root}/site/favicon.svg" "${out}/"

books=()

# Help books, the three guides first and in reading order.
mapfile -t book_dirs < <(
  for name in user admin developer; do
    [[ -d "${root}/docs/${name}.book" ]] && echo "${root}/docs/${name}.book"
  done
  find "${root}/docs" -mindepth 1 -maxdepth 1 -type d -name '*.book' \
    -not -name user.book -not -name admin.book -not -name developer.book | sort
)
for book in "${book_dirs[@]}"; do
  [[ -f "${book}/SUMMARY.md" ]] || continue
  name="$(basename "${book}" .book)"
  title="$(yaml_title "${book}/book.yaml" 2>/dev/null || true)"
  mkdir -p "${work}/src"
  cp -R "${book}" "${work}/src/${name}"
  build_book "${name}" "${title:-${name}}" "${work}/src/${name}"
done

# Architecture decision records, as a generated book.
adrs=("${root}"/docs/adr/[0-9]*.md)
if [[ -e "${adrs[0]}" ]]; then
  src="${work}/src/decisions"
  mkdir -p "${src}"
  {
    echo "# Summary"
    echo
    echo "[Architecture decisions](README.md)"
    echo
    for adr in "${adrs[@]}"; do
      # 0000-template.md is the blank form for new records, not a decision.
      [[ "$(basename "${adr}")" == 0000-* ]] && continue
      cp "${adr}" "${src}/"
      echo "- [$(yaml_title "${adr}")]($(basename "${adr}"))"
    done
  } >"${src}/SUMMARY.md"
  cat >"${src}/README.md" <<'EOF'
# Architecture decisions

Every lasting decision about Vellum is recorded here as an architecture
decision record: the context, the decision and its consequences. Decision
numbers (D1, D2 and so on) are unique across all records.
EOF
  build_book decisions "Vellum architecture decisions" "${src}"
fi

# The documentation cards on the landing page.
describe() {
  case "$1" in
    user) echo "Write pages and books, organise the drive, track work and plan your day." ;;
    admin) echo "Install, configure, secure, back up, upgrade and monitor a deployment." ;;
    developer) echo "The architecture, the Markdown dialect, the APIs, the MCP server and how to contribute." ;;
    decisions) echo "Why Vellum is built the way it is, one decision record at a time." ;;
    *) echo "Read the book." ;;
  esac
}
cards="${work}/cards.html"
if ((${#books[@]} == 0)); then
  echo '        <p>The help books are being written. Until they are published here, the <a href="https://github.com/cruisinme30/vellum">repository</a> is the best guide.</p>' >"${cards}"
else
  {
    echo '        <ul class="cards">'
    for entry in "${books[@]}"; do
      name="${entry%%$'\t'*}"
      title="$(html_escape "${entry#*$'\t'}")"
      printf '          <li><a class="card" href="docs/%s/"><h3>%s</h3><p>%s</p></a></li>\n' \
        "${name}" "${title}" "$(describe "${name}")"
    done
    echo '        </ul>'
  } >"${cards}"
fi
awk -v cards="${cards}" '
  /<!-- BOOKS -->/ { while ((getline line < cards) > 0) print line; next }
  { print }
' "${root}/site/index.html" >"${out}/index.html"

echo "Built the site in ${out} with ${#books[@]} book(s)."
