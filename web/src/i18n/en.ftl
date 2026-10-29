# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only

## Shell

skip-to-content = Skip to content
nav-label = Main
nav-new = New
nav-home = Home
nav-search = Search
nav-favorites = Favorites
nav-inbox = Inbox
nav-data = Data
nav-theme = Theme
nav-help = Help
nav-admin = Admin
nav-more = More
nav-more-label = More destinations
nav-show-pages = Show pages
nav-hide-pages = Hide pages
sidebar-label = Pages
space-add-page = Add a page to { $space }
tree-label = { $space } pages
tree-rename = Page title
tree-expand = Expand { $title }
tree-collapse = Collapse { $title }
menu-label = Actions for { $title }
menu-add-child = Add subpage
menu-rename = Rename
menu-star = Add to favorites
menu-unstar = Remove from favorites
menu-move-up = Move up
menu-move-down = Move down
menu-indent = Nest under the page above
menu-outdent = Move out one level
menu-trash = Move to trash

## Theme

theme-title = Theme
theme-legend = Theme
theme-system = Match the system
theme-light = Light
theme-dark = Dark
theme-high-contrast = High contrast
theme-sepia = Sepia
theme-dim = Dim
theme-seed = Accent colour
theme-seed-reset = Reset colour
density-legend = Density
density-comfortable = Comfortable
density-compact = Compact

## Views

home-greeting = Good to see you, { $name }
home-recent = Recently changed
home-recent-empty = Nothing has changed yet.
favorites-empty = Star a page to keep it here.
search-label = Search pages
search-hint = Type to search every page title and body.
search-count =
    { $count ->
        [one] One page found
       *[other] { $count } pages found
    }
search-empty = No page matches “{ $query }”.
inbox-intro = Pages that mention @{ $handle }.
inbox-empty = Nobody has mentioned you yet.
admin-pending = Workspace administration opens here when the identity and admin services land.
help-shortcuts = Keyboard shortcuts
help-blocks = Blocks you can insert with /
help-key-slash = Insert any block
help-key-format = Bold, italic, code
help-key-undo = Undo, redo
help-key-indent = Indent or outdent a list item
help-key-inline = Typed as text, they become chips
help-key-tree = Reorder and nest pages in the page tree
help-key-menu = Open a page’s menu in the page tree
help-key-grid = Move between table cells, edit one, open its row
help-key-board = Move a board card to the next column
not-found-title = This page does not exist
not-found-home = Go home

## Page

breadcrumbs = Breadcrumbs
page-title = Page title
page-untitled = Untitled
page-star = Favorite
page-status = Status
page-owner = Owner
page-updated = Updated
status-draft = Draft
status-in-review = In review
status-verified = Verified
status-outdated = Outdated

## Editor

editor-label = Page body
editor-placeholder = Type / to insert a block
toolbar-label = Formatting
toolbar-bold = Bold
toolbar-italic = Italic
toolbar-strike = Strikethrough
toolbar-code = Inline code
toolbar-highlight = Highlight
toolbar-link = Link
toolbar-link-prompt = Link address (leave empty to remove the link)
toolbar-insert = Insert
slash-label = Blocks
slash-empty = No block matches.
details-expand = Expand section
details-collapse = Collapse section
details-summary = Section title
tabs-label = Tabs
tabs-add = Add a tab
tabs-title = Tab title
tabs-untitled = Untitled tab
tabs-new-title = Tab { $n }
toc-label = Table of contents
toc-empty = Headings below this block appear here.
field-url = Address
field-title = Title
field-image-url = Image address
field-alt = Alternative text
link-empty = Add the address of the page to link.
embed-empty = Add the address of a video, design or map.
embed-title = { $provider } embed
embed-unsupported = Open { $url } (this site cannot be embedded)
image-empty = Add an image address.
fence-args = { $lang } options
fence-args-placeholder = Options
fence-source = { $lang } source
fence-chart-invalid = The chart needs a values list.
fence-rendered = The { $lang } block is drawn by the renderer when the page is saved.
fence-kroki = The { $lang } diagram is drawn by the diagram service when it is enabled; otherwise the source is shown.
chart-summary = { $title }: { $summary }
editor-loading = Opening the page…

## Collections

collections-intro = Collections are folders of pages with properties, shown as tables, boards, galleries, calendars, timelines and lists.
collections-empty = There are no collections yet.
collections-summary =
    { $rows ->
        [one] One row
       *[other] { $rows } rows
    } · { $views ->
        [one] one view
       *[other] { $views } views
    }
collection-loading = Loading…
collection-missing = This collection does not exist
collection-back = Back to all collections
collection-views = Views
collection-new-row = New row
collection-empty = No rows match this view.
collection-truncated = Showing { $shown } of { $total } rows.
collection-table-hint = Arrow keys move between cells. Enter or F2 edits a cell, Enter saves, Escape cancels, Shift+Enter opens the row.
collection-edit-cell = { $property } of { $row }
collection-open-row = Open { $title }
collection-yes = Yes
collection-no = No
collection-none = None
collection-no-choices = Nothing to choose from.
collection-error-number = Enter a number, such as 3 or 2.5.
collection-error-date = Enter a date as YYYY-MM-DD.
collection-error-url = Enter a full address, such as https://example.com.
collection-board-hint = Drag a card to another column, or press Alt+Left or Alt+Right on it. Each card’s move button lists every column.
collection-move-label = Move { $title }
collection-move-to = Move to { $group }
collection-moved = Moved { $title } to { $group }.
collection-previous-month = Previous month
collection-next-month = Next month
collection-this-month = This month
collection-undated =
    { $count ->
        [one] One row has no date.
       *[other] { $count } rows have no date.
    }
collection-span = from { $start } to { $end }
collection-no-dates = No dates
collection-computed = Computed, read-only
collection-body = Notes
collection-close-row = Close row
