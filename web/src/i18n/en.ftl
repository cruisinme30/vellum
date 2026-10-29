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
search-working = Searching…
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
workspace-loading = Loading your pages…
workspace-problem-request = { $message } (request { $request })
workspace-problem-dismiss = Dismiss

## Page

breadcrumbs = Breadcrumbs
page-title = Page title
page-untitled = Untitled
page-star = Favorite
page-status = Status
page-owner = Owner
page-updated = Updated
folder-intro = A folder. The pages in it:
folder-empty = This folder is empty.
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

## Sign-in and first-run set-up

auth-email = Email
auth-password = Password
auth-error = Something went wrong. Try again.
auth-loading = Connecting to Vellum…
auth-sign-out = Sign out
auth-signed-in-as = Signed in as { $name } ({ $email })
sign-in-title = Sign in
sign-in-intro = Sign in to your Vellum workspace.
sign-in-submit = Sign in
setup-title = Set up your workspace
setup-intro = This workspace is new. Create the first administrator account; you can invite everyone else afterwards.
setup-name = Your name
setup-password-hint = At least 12 characters.
setup-submit = Create account

## Notification settings

notify-open = Notification settings
notify-title = Notifications
notify-intro = Choose what reaches you and where. These settings follow you to every device.
notify-loading = Loading your notification settings…
notify-retry = Try again
notify-save = Save
notify-discard = Discard changes
notify-saved = Saved.
notify-channels = What reaches you
notify-channels-caption = Each row is a kind of notification; each column is a place it can reach you.
notify-kind = Notification
notify-channel-in_app = Inbox
notify-channel-email = Email
notify-channel-push = Push
notify-toggle = { $channel } for { $kind }
notify-urgent = Urgent
notify-kind-issue_assigned = Assigned to you
notify-kind-mentioned = Mentions
notify-kind-commented = Comments on things you follow
notify-kind-state_changed = Status changes
notify-kind-triage_new = New in your triage inbox
notify-kind-sla_high_risk = SLA at risk
notify-kind-sla_breached = SLA breached
notify-kind-review_requested = Review requests
notify-kind-project_update = Project updates
notify-kind-reminder = Reminders
notify-kind-pull_request = Pull requests
notify-email = Email timing
notify-email-delay = Send email after
notify-email-now = Straight away
notify-minutes = { $n ->
        [one] 1 minute
       *[other] { $n } minutes
    }
notify-email-hint = If you read it in Vellum first, the email is not sent. Urgent notifications are emailed straight away.
notify-snooze = Snooze
notify-snooze-hint = Hold push and email for a while. Your inbox still fills up, and urgent notifications still come through.
notify-snooze-1h = 1 hour
notify-snooze-8h = 8 hours
notify-snooze-1d = 1 day
notify-snooze-1w = 1 week
notify-snoozed-until = Snoozed until { $time }.
notify-resume = Resume now
notify-muted = Muted
notify-muted-empty = Nothing is muted.
notify-unmute = Unmute
notify-unmute-label = Unmute { $subject }
notify-mute-subject = Issue key or page ID
notify-mute = Mute
notify-device = This device
notify-device-checking = Checking this browser…
notify-device-on = Push notifications are on for this browser.
notify-device-off = Push notifications are off for this browser.
notify-device-denied = This browser blocked notifications. Allow them in the site settings, then try again.
notify-device-unsupported = This browser can't receive push notifications. On iPhone and iPad, add Vellum to your Home Screen first.
notify-device-demo = Push notifications need a Vellum server; the demo can't send them.
notify-device-enable = Turn on push
notify-device-disable = Turn off push
notify-device-error = Push couldn't be changed. Try again.
