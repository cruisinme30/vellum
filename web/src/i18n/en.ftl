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
admin-intro = Run this Vellum server: who did what, settings, backups and how requests perform.
admin-audit = Audit log
admin-audit-summary = Who did what, in a tamper-evident log you can verify and export.
admin-settings = Settings
admin-settings-summary = Backups, admin sessions, observability and anonymous telemetry.
admin-backups = Backups
admin-backups-summary = The last backup and restore drill, and both on demand.
admin-requests = Requests
admin-requests-summary = Recent requests with every span, to see why one was slow or failed.
admin-routes = Routes
admin-routes-summary = Rate, errors and latency per route, against its budget.
admin-needs-server = This screen needs a Vellum server. The demo has nothing to administer.
admin-loading = Loading…
admin-older = Older
admin-newer = Newer
admin-filter = Filter
admin-save = Save
admin-saving = Saving…
admin-reload = Load the latest settings
admin-outcome-success = Succeeded
admin-outcome-denied = Denied
admin-outcome-failure = Failed
admin-audit-intro = Every admin action, sign-in and permission change, newest first. Each record holds the hash of the one before, so any edit to the log shows.
admin-audit-empty = No records match.
admin-audit-caption = Audit records, newest first
admin-audit-when = When
admin-audit-who = Who
admin-audit-action = Action
admin-audit-target = Target
admin-audit-outcome = Outcome
admin-audit-request = Request
admin-audit-user = User ID
admin-audit-verify = Verify the log
admin-audit-export = Export
admin-audit-download = Download the export
admin-audit-intact = The log is intact: { $records } records checked.
admin-audit-broken = The log was altered at record { $seq }: { $reason }
admin-settings-intro = Changes apply at once. If another admin saves first, reload and make your change again.
admin-settings-saved = Settings saved.
admin-settings-conflict = Another admin saved these settings since you opened them. Nothing was changed.
admin-settings-backups = Backups
admin-settings-security = Admin sessions
admin-settings-observability = Observability
admin-settings-telemetry = Anonymous telemetry
admin-setting-organisation = Organisation name
admin-setting-backups-enabled = Run scheduled backups
admin-setting-backups-interval-hours = Hours between backups
admin-setting-backups-keep = Backups to keep
admin-setting-backups-destination = Backup folder
admin-setting-backups-drill-interval-days = Days between restore drills
admin-setting-security-fresh-auth-minutes = Admin actions need a sign-in within (minutes)
admin-setting-security-session-hours = Session length (hours)
admin-setting-observability-slow-request-ms = Highlight requests slower than (ms)
admin-setting-observability-trace-buffer-capacity = Requests kept for Admin → Requests
admin-setting-observability-trace-buffer-mib = Memory for kept requests (MiB)
admin-setting-telemetry-enabled = Send anonymous usage counts
admin-setting-telemetry-endpoint = Report address (HTTPS)
admin-setting-telemetry-interval-hours = Hours between reports
admin-backups-intro = Backups are encrypted. A restore drill restores the newest one into scratch space and checks every file.
admin-backups-now = Back up now
admin-backups-running = Backing up…
admin-backups-run-drill = Run a restore drill
admin-backups-drilling = Running the drill…
admin-backups-made = Backup { $name } written ({ $size }).
admin-backups-drill-ok = The restore drill passed: { $files } files checked.
admin-backups-drill-bad = The restore drill failed: { $error }
admin-backups-last = Last backup
admin-backups-last-detail = { $when }, { $files } files, { $size }
admin-backups-drill = Last restore drill
admin-backups-drill-passed = Passed { $when } on { $backup } ({ $files } files).
admin-backups-drill-failed = Failed { $when } on { $backup }: { $error }
admin-backups-telemetry = Telemetry last sent
admin-backups-never = Never
admin-backups-empty = No backups yet.
admin-backups-caption = Backups kept, newest first
admin-backups-file = File
admin-backups-size = Size
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
editor-connecting = Connecting to the server…
editor-offline = Offline. Your changes are kept and sync when the connection returns.
editor-read-only = You can view this page but not edit it.
editor-revoked = Your access to this page was removed. Reload to see what you can open.
editor-saved = Saved
editor-unsaved = Unsaved changes
editor-saving = Saving…
editor-save-failed = Not saved yet. Your changes are kept and saving will retry.
editor-done = Done
editor-done-hint = Save a version now (Ctrl+S)
history-open = Version history
history-title = Version history
history-close = Close version history
history-loading = Loading…
history-empty = No saved versions yet. A version is saved after 30 seconds without edits, or when you press Done.
history-versions = Saved versions, newest first
history-current = current
history-preview = Version text
history-not-text = This version is not text, so it cannot be shown.
history-by-you = You
history-by-person = Another member
history-by-model = An AI assistant
history-by-external = Edited outside Vellum
history-by-system = Vellum
history-restore = Restore this version
history-restore-explain = The page's text becomes this version again. Your current text is saved as a version first, so you can come back to it.
history-restore-confirm = Restore
history-restoring = Restoring…
history-cancel = Cancel
history-restored = Restored the version from { $when }.
history-save-first-failed = Your latest changes could not be saved, so nothing was restored. Try again in a moment.

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

## Private spaces (end-to-end encrypted)

private-title = Private spaces
private-intro = Pages in a private space are sealed on members' devices. The server stores only ciphertext, so it can't read, search or export them.
private-device = This device
private-device-desktop-only = Enrolling a device needs the Vellum desktop app, which keeps the device key in this computer's keychain.
private-device-fingerprint = Device fingerprint:
private-no-spaces = You aren't in any shared drives yet.
private-not-private = Not private. Pages are encrypted at rest with the server's keys.
private-sealed-to = { $count ->
    [one] Sealed to 1 device.
   *[other] Sealed to { $count } devices.
}
private-this-device = (this device)
private-make-private = Make private and enrol this device
private-enrol = Enrol this device
private-reseal = Reseal pages for every device
private-remove = Remove
private-remove-label = Remove { $device }
private-removed = Removed { $device }. Reseal the pages from another device so it can't open new versions.
private-error = That didn't work. Try again.

## Screens that need a server

needs-server = This screen needs a Vellum server. The demo has no data for it.
loading = Loading…

## My Work

my-work = My Work
my-work-intro = Your tasks and the issues assigned to you.
my-work-add = Add a task
my-work-add-hint = Call Sam tomorrow 3pm !high
my-work-add-button = Add
my-work-empty = Nothing to do. Add a task above.
my-work-due = Due { $date }
my-work-bucket-inbox = Inbox
my-work-bucket-today = Today
my-work-bucket-evening = This evening
my-work-bucket-upcoming = Upcoming
my-work-bucket-anytime = Anytime
my-work-bucket-someday = Someday
my-work-bucket-done = Done

## Issues

issues = Issues
issues-intro = Each team's open issues.
issues-team = Team
issues-no-teams = There are no teams yet. An admin can add one.
issues-empty = This team has no open issues.
issues-new = New issue for { $team }
issues-create = Create issue
issue-state = State: { $state }
issue-state-label = State
issue-no-description = No description.
issue-comments = Comments
issue-no-comments = No comments yet.
issue-comment-by = { $author }, { $when }
issue-comment-add = Add a comment
issue-comment-send = Comment

## Books

books = Books
books-intro = Books you can read.
books-empty = There are no books you can read.
books-contents = Contents
books-no-sections = This book has no sections yet.
home-shortcuts = Shortcuts
