// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Generated from the server's operations by `vellum openapi --typescript`.
// Do not edit: run `VELLUM_BLESS=1 cargo test -p vellum-cli typescript_client`.

/**
 * An item's activity.
 */
export interface ActivityInput {
  /**
   * The item, or a space's ID for the whole space.
   */
  id: ObjectId;
  /**
   * At most this many, 1 to 200; 50 when absent.
   */
  limit?: number | null;
}

/**
 * An activity feed, newest first.
 */
export interface ActivityList {
  /**
   * The changes.
   */
  events: ActivityView[];
}

/**
 * One change in the activity feed.
 */
export interface ActivityView {
  /**
   * Who did it.
   */
  actor: ActorView;
  /**
   * When it happened.
   */
  at: Timestamp;
  /**
   * The old name or path, for renames and moves.
   */
  from?: string | null;
  /**
   * The item it happened to.
   */
  item_id: ObjectId;
  /**
   * What happened: `created`, `edited`, `renamed`, `moved`, `copied`,
   * `trashed`, `restored`, `purged`, `conflict_copy` or
   * `version_restored`.
   */
  kind: string;
  /**
   * The new name or path, for renames and moves.
   */
  to?: string | null;
}

/**
 * Who made a change.
 */
export type ActorView = {
  type: "user";
  /**
   * The person.
   */
  user: UserId;
} | {
  /**
   * The person.
   */
  on_behalf_of: UserId;
  type: "model";
} | {
  type: "external";
} | {
  type: "system";
};

/**
 * A new comment or reply.
 */
export interface AddComment {
  /**
   * "Assign to" this handle.
   */
  assign_to?: string | null;
  /**
   * Body in Vellum Markdown.
   */
  body: string;
  /**
   * Written by a model for the user (MCP); shown with a badge.
   */
  by_model?: boolean;
  /**
   * The selected text, for a block thread.
   */
  quote?: string | null;
  /**
   * The comment to reply to.
   */
  reply_to?: string | null;
  /**
   * Where to start a thread: an issue key, a document ID or
   * `<doc id>#blk-…`. Leave unset for a reply.
   */
  target?: string | null;
}

/**
 * `books.add_section`.
 */
export interface AddSection {
  /**
   * Its Markdown body.
   */
  body?: string;
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * Where it goes.
   */
  place: BooksAddSectionPlaceInput;
  /**
   * The new section's title.
   */
  title: string;
}

/**
 * No input.
 */
export type AdminAppearanceEmpty = Record<string, unknown>;

/**
 * A finished backup.
 */
export interface AdminCreateBackupBackupInfo {
  /**
   * Plaintext bytes.
   */
  bytes: number;
  /**
   * When it started.
   */
  created_at: Timestamp;
  /**
   * How long it took, in milliseconds.
   */
  duration_ms: number;
  /**
   * Files included.
   */
  files: number;
  /**
   * The backup file name.
   */
  name: string;
  /**
   * Size of the backup file.
   */
  size: number;
  /**
   * Symbolic links skipped (never followed).
   */
  skipped_links: number;
}

/**
 * No input.
 */
export type AdminCreateBackupEmpty = Record<string, unknown>;

/**
 * No input.
 */
export type AdminExportAuditEmpty = Record<string, unknown>;

/**
 * The organisation's look (R-TH-1, R-TH-4; ADR 0007 D245).
 */
export interface AdminGetSettingsAppearanceSettings {
  /**
   * The theme exports (PDF, HTML, Word) use: a light theme, since
   * paper is white.
   */
  export_theme?: string;
  /**
   * The seed colour every built-in theme is derived from, `#rrggbb`.
   * People can still pick their own accent in the app.
   */
  seed?: string;
  /**
   * The theme of published sites; `auto` follows the reader's system
   * light or dark setting.
   */
  site_theme?: string;
}

/**
 * No input.
 */
export type AdminGetSettingsEmpty = Record<string, unknown>;

/**
 * No input.
 */
export type AdminListBackupsEmpty = Record<string, unknown>;

/**
 * The outcome of a restore drill.
 */
export interface AdminRunDrillDrillReport {
  /**
   * The backup drilled.
   */
  backup: string;
  /**
   * Bytes verified.
   */
  bytes: number;
  /**
   * Restore time in milliseconds.
   */
  duration_ms: number;
  /**
   * Why the drill failed, when it did.
   */
  error?: string | null;
  /**
   * Files verified.
   */
  files: number;
  /**
   * Whether every file restored and matched the manifest within the RTO.
   */
  passed: boolean;
  /**
   * When the drill ran.
   */
  ran_at: Timestamp;
  /**
   * Whether the restore finished within [`RTO_SECONDS`].
   */
  within_rto: boolean;
}

/**
 * No input.
 */
export type AdminRunDrillEmpty = Record<string, unknown>;

/**
 * Every admin setting.
 */
export interface AdminSettings {
  /**
   * The organisation's seed colour and export and site themes.
   */
  appearance?: AdminGetSettingsAppearanceSettings;
  /**
   * Backups.
   */
  backups?: BackupSettings;
  /**
   * Built-in observability.
   */
  observability?: ObservabilitySettings;
  /**
   * The organisation's display name.
   */
  organisation?: string;
  /**
   * Admin-session security.
   */
  security?: SecuritySettings;
  /**
   * Anonymous usage reporting.
   */
  telemetry?: TelemetrySettings;
}

/**
 * No input.
 */
export type AdminSlosEmpty = Record<string, unknown>;

/**
 * Persistent scheduler state, kept in `.vellum/admin/state.json`.
 */
export interface AdminState {
  /**
   * The last successful backup.
   */
  last_backup?: BackupInfo | null;
  /**
   * The last restore drill, passed or not.
   */
  last_drill?: DrillReport | null;
  /**
   * When anonymous telemetry was last sent, Unix milliseconds.
   */
  last_telemetry_ms?: number | null;
}

/**
 * No input.
 */
export type AdminStateEmpty = Record<string, unknown>;

/**
 * No input.
 */
export type AdminTelemetryPreviewEmpty = Record<string, unknown>;

/**
 * Every admin setting.
 */
export interface AdminUpdateSettingsAdminSettings {
  /**
   * The organisation's seed colour and export and site themes.
   */
  appearance?: AdminUpdateSettingsAppearanceSettings;
  /**
   * Backups.
   */
  backups?: BackupSettings;
  /**
   * Built-in observability.
   */
  observability?: ObservabilitySettings;
  /**
   * The organisation's display name.
   */
  organisation?: string;
  /**
   * Admin-session security.
   */
  security?: SecuritySettings;
  /**
   * Anonymous usage reporting.
   */
  telemetry?: AdminUpdateSettingsTelemetrySettings;
}

/**
 * Every admin setting.
 */
export interface AdminUpdateSettingsAdminSettings2 {
  /**
   * The organisation's seed colour and export and site themes.
   */
  appearance?: AdminUpdateSettingsAppearanceSettings2;
  /**
   * Backups.
   */
  backups?: BackupSettings;
  /**
   * Built-in observability.
   */
  observability?: ObservabilitySettings;
  /**
   * The organisation's display name.
   */
  organisation?: string;
  /**
   * Admin-session security.
   */
  security?: SecuritySettings;
  /**
   * Anonymous usage reporting.
   */
  telemetry?: AdminUpdateSettingsTelemetrySettings2;
}

/**
 * The organisation's look (R-TH-1, R-TH-4; ADR 0007 D245).
 */
export interface AdminUpdateSettingsAppearanceSettings {
  /**
   * The theme exports (PDF, HTML, Word) use: a light theme, since
   * paper is white.
   */
  export_theme?: string;
  /**
   * The seed colour every built-in theme is derived from, `#rrggbb`.
   * People can still pick their own accent in the app.
   */
  seed?: string;
  /**
   * The theme of published sites; `auto` follows the reader's system
   * light or dark setting.
   */
  site_theme?: string;
}

/**
 * The organisation's look (R-TH-1, R-TH-4; ADR 0007 D245).
 */
export interface AdminUpdateSettingsAppearanceSettings2 {
  /**
   * The theme exports (PDF, HTML, Word) use: a light theme, since
   * paper is white.
   */
  export_theme?: string;
  /**
   * The seed colour every built-in theme is derived from, `#rrggbb`.
   * People can still pick their own accent in the app.
   */
  seed?: string;
  /**
   * The theme of published sites; `auto` follows the reader's system
   * light or dark setting.
   */
  site_theme?: string;
}

/**
 * Opt-in anonymous usage reporting.
 */
export interface AdminUpdateSettingsTelemetrySettings {
  /**
   * Whether reports are sent. Off until an admin turns it on.
   */
  enabled?: boolean;
  /**
   * Where reports go (HTTPS).
   */
  endpoint?: string | null;
  /**
   * A random id for this installation, unrelated to anything else.
   */
  install_id?: string;
  /**
   * Hours between reports.
   */
  interval_hours?: number;
}

/**
 * Opt-in anonymous usage reporting.
 */
export interface AdminUpdateSettingsTelemetrySettings2 {
  /**
   * Whether reports are sent. Off until an admin turns it on.
   */
  enabled?: boolean;
  /**
   * Where reports go (HTTPS).
   */
  endpoint?: string | null;
  /**
   * A random id for this installation, unrelated to anything else.
   */
  install_id?: string;
  /**
   * Hours between reports.
   */
  interval_hours?: number;
}

/**
 * Settings with the version they were stored at.
 */
export interface AdminUpdateSettingsVersioned {
  /**
   * The settings.
   */
  settings: AdminUpdateSettingsAdminSettings2;
  /**
   * Increases by one on every save; 0 before the first.
   */
  version: number;
}

/**
 * No input.
 */
export type AdminVerifyAuditEmpty = Record<string, unknown>;

/**
 * How rollups combine the related rows' values.
 */
export type Aggregation = "count" | "sum" | "avg" | "min" | "max" | "checked" | "pct" | "unique" | "show";

export type AnyValue = unknown;

/**
 * The JSON body of every error response, REST and GraphQL alike
 * (`extensions` in GraphQL). Clients show `message` and offer
 * `request_id` as the "copy diagnostic" value.
 */
export interface ApiError {
  /**
   * A stable `snake_case` code: `invalid_input`, `unauthenticated`,
   * `permission_denied`, `not_found`, `conflict`, `rate_limited`,
   * `unavailable`, `timeout` or `internal`.
   */
  code: string;
  /**
   * A message safe to show to the user.
   */
  message: string;
  /**
   * The request ID, as in the `X-Request-Id` header.
   */
  request_id: string;
  /**
   * Whether retrying the same request later may succeed.
   */
  retryable: boolean;
}

/**
 * A connected `OAuth` app.
 */
export interface AppView {
  /**
   * The resources it may call.
   */
  audience: string[];
  /**
   * The client ID.
   */
  clientId: string;
  /**
   * The client's name.
   */
  clientName: string;
  /**
   * When it was approved.
   */
  createdAt: Timestamp;
  /**
   * The grant ID.
   */
  id: string;
  /**
   * The granted scopes.
   */
  scopes: Scope[];
}

/**
 * The organisation's look (R-TH-1, R-TH-4; ADR 0007 D245).
 */
export interface AppearanceSettings {
  /**
   * The theme exports (PDF, HTML, Word) use: a light theme, since
   * paper is white.
   */
  export_theme?: string;
  /**
   * The seed colour every built-in theme is derived from, `#rrggbb`.
   * People can still pick their own accent in the app.
   */
  seed?: string;
  /**
   * The theme of published sites; `auto` follows the reader's system
   * light or dark setting.
   */
  site_theme?: string;
}

export type Array_of_AppView = AppView[];

export type Array_of_SessionView = SessionView[];

export type Array_of_TokenView = TokenView[];

export type Array_of_UserView = UserView[];

/**
 * Assign a thread.
 */
export interface AssignThread {
  /**
   * Handle; unset or empty unassigns.
   */
  assignee?: string | null;
  /**
   * Thread ID.
   */
  thread_id: string;
}

/**
 * One "assign to" item.
 */
export interface AssignmentOut {
  /**
   * When.
   */
  assigned_at?: string | null;
  /**
   * Who assigned it.
   */
  assigned_by?: string | null;
  /**
   * The quoted text or the start of the comment.
   */
  summary: string;
  /**
   * Where it is.
   */
  target: TargetOut;
  /**
   * Thread ID.
   */
  thread: string;
}

/**
 * Open "assign to" items.
 */
export interface AssignmentsOut {
  /**
   * Oldest first.
   */
  assignments: AssignmentOut[];
}

/**
 * Where to put a section.
 */
export type At = "before" | "after" | "first_child_of" | "last_child_of" | "start" | "end";

/**
 * Who acted, as recorded. Mirrors `vellum_kernel::Actor` with the API
 * token or MCP client when there is one.
 */
export interface AuditActor {
  /**
   * `user`, `model`, `api_token`, `break_glass`, `external` or `system`.
   */
  kind: string;
  /**
   * The person, when there is one.
   */
  user_id?: string | null;
  /**
   * The API token, MCP client or session that carried the call.
   */
  via?: string | null;
}

/**
 * The audit log as JSON Lines.
 */
export interface AuditExport {
  /**
   * One record per line, oldest first.
   */
  jsonl: string;
}

/**
 * Audit records, newest first.
 */
export interface AuditList {
  /**
   * The records.
   */
  records: AuditRecord[];
}

/**
 * Filters for [`AuditLog::list`].
 */
export interface AuditQuery {
  /**
   * Only this action.
   */
  action?: string | null;
  /**
   * Only records before this sequence number (paging cursor).
   */
  before?: number | null;
  /**
   * At most this many records (default 100, capped at [`MAX_LIST`]).
   */
  limit?: number | null;
  /**
   * Only this person.
   */
  user_id?: string | null;
}

/**
 * A stored, chained record.
 */
export interface AuditRecord {
  /**
   * When it was appended.
   */
  at: Timestamp;
  /**
   * The event.
   */
  event: NewAuditEvent;
  /**
   * SHA-256 over `prev_hash` and this record's canonical JSON.
   */
  hash: string;
  /**
   * The previous record's hash, or [`GENESIS`].
   */
  prev_hash: string;
  /**
   * Position in the log, from 1.
   */
  seq: number;
}

/**
 * Who is calling and what the workspace needs.
 */
export interface AuthStatus {
  /**
   * How the caller authenticated.
   */
  credential: string;
  /**
   * Whether the workspace still needs its first administrator.
   */
  needsSetup: boolean;
  /**
   * The caller's scopes.
   */
  scopes: Scope[];
  /**
   * The single sign-on button, when a provider is configured.
   */
  sso?: SsoButton | null;
  /**
   * The signed-in user, if any.
   */
  user?: UserView | null;
}

/**
 * A backup file on disk.
 */
export interface BackupFile {
  /**
   * The file name.
   */
  name: string;
  /**
   * Its size in bytes.
   */
  size: number;
}

/**
 * A finished backup.
 */
export interface BackupInfo {
  /**
   * Plaintext bytes.
   */
  bytes: number;
  /**
   * When it started.
   */
  created_at: Timestamp;
  /**
   * How long it took, in milliseconds.
   */
  duration_ms: number;
  /**
   * Files included.
   */
  files: number;
  /**
   * The backup file name.
   */
  name: string;
  /**
   * Size of the backup file.
   */
  size: number;
  /**
   * Symbolic links skipped (never followed).
   */
  skipped_links: number;
}

/**
 * The backups on disk, newest first.
 */
export interface BackupList {
  /**
   * The backups.
   */
  backups: BackupFile[];
}

/**
 * Scheduled encrypted backups.
 */
export interface BackupSettings {
  /**
   * Directory backups are written to; relative paths are under the
   * data directory.
   */
  destination?: string;
  /**
   * Days between automatic restore drills (brief §11: monthly).
   */
  drill_interval_days?: number;
  /**
   * Whether scheduled backups run.
   */
  enabled?: boolean;
  /**
   * Hours between backups.
   */
  interval_hours?: number;
  /**
   * Backups kept; older ones are deleted.
   */
  keep?: number;
}

/**
 * A block ID: blk- and 8 Crockford base32 characters.
 */
export type BlockId = string;

/**
 * The kind of a block, which shapes how its hit is shown.
 */
export type BlockKind = "heading" | "paragraph" | "list_item" | "task" | "quote" | "code" | "table" | "callout" | "caption" | "property" | "other";

/**
 * A BookId as a 26-character ULID.
 */
export type BookId = string;

/**
 * A book as the caller sees it.
 */
export interface BookInfo {
  /**
   * The book.
   */
  id: BookId;
  /**
   * The deepest nesting allowed.
   */
  max_depth: number;
  /**
   * `decimal`, `outline` or `none`.
   */
  numbering: string;
  /**
   * The caller's role on the book: `viewer` to `owner`.
   */
  role: string;
  /**
   * Its title.
   */
  title: string;
  /**
   * The table of contents, without the sections the caller cannot see.
   */
  toc: TocLine[];
}

/**
 * Names a book.
 */
export interface BookKey {
  /**
   * The book.
   */
  book_id: BookId;
}

/**
 * The books the caller can open, by title.
 */
export interface BookList {
  /**
   * The books.
   */
  books: BookSummary[];
}

/**
 * A book in a list.
 */
export interface BookSummary {
  /**
   * The book.
   */
  id: BookId;
  /**
   * The `.book` folder.
   */
  path: string;
  /**
   * Its title.
   */
  title: string;
}

/**
 * A place in the tree: relative to a section, or at either end of a
 * region.
 */
export interface BooksAddSectionPlaceInput {
  /**
   * Where.
   */
  at: At;
  /**
   * The region `start` and `end` refer to; `numbered` by default.
   */
  region?: Region | null;
  /**
   * The section `before`, `after`, `first_child_of` and
   * `last_child_of` refer to.
   */
  section?: string | null;
}

/**
 * A snapshot's header.
 */
export interface BooksCreateSnapshotSnapshotInfo {
  /**
   * When it was taken (RFC 3339).
   */
  created_at: string;
  /**
   * Who took it (`user:<id>`).
   */
  created_by: string;
  /**
   * What it marks.
   */
  message: string;
  /**
   * The tag, such as `v1.0`.
   */
  tag: string;
}

/**
 * A place in the tree: relative to a section, or at either end of a
 * region.
 */
export interface BooksMoveSectionPlaceInput {
  /**
   * Where.
   */
  at: At;
  /**
   * The region `start` and `end` refer to; `numbered` by default.
   */
  region?: Region | null;
  /**
   * The section `before`, `after`, `first_child_of` and
   * `last_child_of` refer to.
   */
  section?: string | null;
}

/**
 * Sections a removal took out.
 */
export interface BooksRemoveSectionRemoved {
  /**
   * The removed sections, the named one first.
   */
  removed: string[];
}

/**
 * Who may see, own and review a section.
 */
export interface BooksSetSectionAccessSectionAccessInfo {
  /**
   * Its own `restrict_to`, as `book.yaml` stores it.
   */
  own_restrict_to?: string[] | null;
  /**
   * Effective owners.
   */
  owners: string[];
  /**
   * Who may see it (`user:<id>`, `group:<id>`), when it is restricted:
   * the intersection of its own and every enclosing restriction.
   */
  restrict_to?: string[] | null;
  /**
   * The sections whose restrictions apply, outermost first.
   */
  restricted_by: string[];
  /**
   * Effective reviewers.
   */
  reviewers: string[];
}

/**
 * What an assistant reads before suggesting a triage decision.
 */
export interface BriefOut {
  /**
   * The start of its description.
   */
  body: string;
  /**
   * The issue's identifier.
   */
  key: string;
  /**
   * Open issues with similar titles, closest first.
   */
  similar: SimilarOut[];
  /**
   * The suggestion already waiting, if any.
   */
  suggestion?: SuggestionOut | null;
  /**
   * The team whose inbox holds it.
   */
  team: string;
  /**
   * Where it could go.
   */
  teams: TeamOptionOut[];
  /**
   * Its title.
   */
  title: string;
}

/**
 * A bucket of the view.
 */
export type BucketName = "inbox" | "today" | "evening" | "upcoming" | "anytime" | "someday" | "done";

/**
 * One breakdown value.
 */
export interface BucketOut {
  /**
   * The value, such as `ENG`, `dev` or `urgent`; empty means "none".
   */
  key?: string | null;
  /**
   * Over the whole range.
   */
  overall: StatsOut;
  /**
   * Per interval, when asked for; every interval is present.
   */
  series: PointOut[];
}

/**
 * The budgets for every route.
 */
export interface Budgets {
  /**
   * Routes with fewer requests in the window are not judged.
   */
  min_requests?: number;
  /**
   * The p95 budget for reads without a rule, in milliseconds.
   */
  read_ms?: number;
  /**
   * Budgets for particular routes.
   */
  routes?: RouteBudget[];
  /**
   * The p95 budget for writes without a rule, in milliseconds.
   */
  write_ms?: number;
}

/**
 * One burn-up point.
 */
export interface BurnOut {
  /**
   * Points completed.
   */
  completed: number;
  /**
   * The day.
   */
  date: string;
  /**
   * Points in scope.
   */
  scope: number;
}

/**
 * An ID in the path.
 */
export interface ById {
  /**
   * The ID.
   */
  id: string;
}

/**
 * Why a computed cell has no value. Shown inline in the cell.
 */
export interface CellError {
  /**
   * What went wrong, for people.
   */
  message: string;
  /**
   * Where in the formula, when the error is in this cell's own formula.
   */
  span?: Span | null;
}

/**
 * The API shape of a [`Cell`]: `{"value": …}` or `{"error": …}`.
 */
export type CellValue = {
  value: Value;
} | {
  error: CellError;
};

/**
 * One cell of a view result.
 */
export interface CellView {
  /**
   * The value as display text: numbers formatted, relations as titles.
   */
  display: string;
  /**
   * The inline error, if the cell failed to compute.
   */
  error?: CellError | null;
  /**
   * The typed value (absent on error).
   */
  value?: Value | null;
}

/**
 * Where and how the chain is broken.
 */
export interface ChainBreak {
  /**
   * What is wrong.
   */
  reason: string;
  /**
   * The sequence number where the problem was found.
   */
  seq: number;
}

/**
 * The result of checking the chain.
 */
export interface ChainReport {
  /**
   * The first problem found, if any.
   */
  broken_at?: ChainBreak | null;
  /**
   * The head hash.
   */
  head: string;
  /**
   * Records checked.
   */
  records: number;
}

/**
 * What an edit touched.
 */
export interface Changeset {
  /**
   * Rows whose files must be deleted.
   */
  deleted: RowRef[];
  /**
   * Rows whose files must be written (created or changed).
   */
  written: RowRef[];
}

/**
 * Data for a chart, ready for the renderer.
 */
export interface ChartData {
  /**
   * The categories, in axis order.
   */
  categories: string[];
  /**
   * The series.
   */
  series: Series[];
  /**
   * The x-axis label.
   */
  xLabel: string;
  /**
   * The y-axis label.
   */
  yLabel: string;
}

/**
 * A chart whose data comes from a collection:
 * `source=collection:<id> x=… y=…` on a `chart` block.
 */
export interface ChartInput {
  /**
   * How values sharing a category combine: `sum`, `count`, `avg`,
   * `min` or `max`.
   */
  agg?: string | null;
  /**
   * A property that splits rows into series.
   */
  series?: string | null;
  /**
   * `collection:<collection id>`.
   */
  source: string;
  /**
   * The view whose rows feed the chart; the first view when absent.
   */
  view?: string | null;
  /**
   * The category property (x axis).
   */
  x: string;
  /**
   * The value property; rows are counted when absent.
   */
  y?: string | null;
}

/**
 * Where to write an issue chip.
 */
export interface ChipOut {
  /**
   * Block ID.
   */
  block: string;
  /**
   * Document ID.
   */
  doc: string;
  /**
   * The key to write.
   */
  key: string;
}

/**
 * A CollectionId as a 26-character ULID.
 */
export type CollectionId = string;

/**
 * A collection's schema and saved views.
 */
export interface CollectionInfo {
  /**
   * Problems found while loading its files.
   */
  diagnostics: Diagnostic[];
  /**
   * The collection's ID.
   */
  id: CollectionId;
  /**
   * The `name.collection` folder.
   */
  path: string;
  /**
   * Properties in column order.
   */
  properties: PropertyInfo[];
  /**
   * Its title.
   */
  title: string;
  /**
   * Saved views in tab order.
   */
  views: ViewInfo[];
}

/**
 * Names one collection.
 */
export interface CollectionKey {
  /**
   * The collection's ID (the `id` in `schema.yaml`).
   */
  collection_id: CollectionId;
}

/**
 * The collections the caller can read.
 */
export interface CollectionList {
  /**
   * Ordered by title.
   */
  collections: CollectionSummary[];
}

/**
 * One collection in a listing.
 */
export interface CollectionSummary {
  /**
   * The collection's ID.
   */
  id: CollectionId;
  /**
   * The `name.collection` folder.
   */
  path: string;
  /**
   * How many rows it has.
   */
  rows: number;
  /**
   * Its title.
   */
  title: string;
  /**
   * Saved view names, in tab order.
   */
  views: string[];
}

/**
 * A collection's schema and saved views.
 */
export interface CollectionsCreateFromCsvCollectionInfo {
  /**
   * Problems found while loading its files.
   */
  diagnostics: Diagnostic[];
  /**
   * The collection's ID.
   */
  id: CollectionId;
  /**
   * The `name.collection` folder.
   */
  path: string;
  /**
   * Properties in column order.
   */
  properties: PropertyInfo[];
  /**
   * Its title.
   */
  title: string;
  /**
   * Saved views in tab order.
   */
  views: ViewInfo[];
}

/**
 * What a CSV import did, or would do on a dry run.
 */
export interface CollectionsCreateFromCsvCsvImported {
  /**
   * Row files written, including links mirrored into other collections.
   */
  changed: Changeset;
  /**
   * The collection after the import: its properties (inferred, for a
   * new collection) and views.
   */
  collection: CollectionsCreateFromCsvCollectionInfo;
  /**
   * Problems with individual cells; their values were kept as text or
   * left empty.
   */
  diagnostics: Diagnostic[];
  /**
   * Rows created (or that would be).
   */
  rows: number;
  /**
   * Headers that matched no writable property and were ignored.
   */
  skippedColumns: string[];
  /**
   * Whether anything was written.
   */
  written: boolean;
}

/**
 * One row with every value, stored and computed.
 */
export interface CollectionsCreateRowRowDetail {
  /**
   * The page's Markdown body.
   */
  body: string;
  /**
   * The collection.
   */
  collectionId: CollectionId;
  /**
   * The row page's path.
   */
  path: string;
  /**
   * Values by property key; computed cells may hold an inline error.
   */
  row: ComputedRow;
  /**
   * The row's title.
   */
  title: string;
}

/**
 * One row with every value, stored and computed.
 */
export interface CollectionsDeleteRowRowDetail {
  /**
   * The page's Markdown body.
   */
  body: string;
  /**
   * The collection.
   */
  collectionId: CollectionId;
  /**
   * The row page's path.
   */
  path: string;
  /**
   * Values by property key; computed cells may hold an inline error.
   */
  row: ComputedRow;
  /**
   * The row's title.
   */
  title: string;
}

/**
 * The result of a row write.
 */
export interface CollectionsDeleteRowRowWritten {
  /**
   * Every row file written or removed, including mirrored relations.
   */
  changed: Changeset;
  /**
   * The row after the write; absent after a delete.
   */
  row?: CollectionsDeleteRowRowDetail | null;
}

/**
 * The result of running a view.
 */
export interface CollectionsEmbedViewResult {
  /**
   * The collection.
   */
  collection: CollectionId;
  /**
   * Visible columns, title first.
   */
  columns: Column[];
  /**
   * Groups; a single unlabelled group when the view is not grouped.
   */
  groups: Group[];
  /**
   * The layout.
   */
  kind: ViewKind;
  /**
   * The collection title.
   */
  title: string;
  /**
   * Rows matching the filters, before the limit.
   */
  total: number;
  /**
   * Whether the limit cut rows off.
   */
  truncated: boolean;
  /**
   * The view name.
   */
  view: string;
}

/**
 * A collection's schema and saved views.
 */
export interface CollectionsImportCsvCollectionInfo {
  /**
   * Problems found while loading its files.
   */
  diagnostics: Diagnostic[];
  /**
   * The collection's ID.
   */
  id: CollectionId;
  /**
   * The `name.collection` folder.
   */
  path: string;
  /**
   * Properties in column order.
   */
  properties: PropertyInfo[];
  /**
   * Its title.
   */
  title: string;
  /**
   * Saved views in tab order.
   */
  views: ViewInfo[];
}

/**
 * One row with every value, stored and computed.
 */
export interface CollectionsUpdateRowRowDetail {
  /**
   * The page's Markdown body.
   */
  body: string;
  /**
   * The collection.
   */
  collectionId: CollectionId;
  /**
   * The row page's path.
   */
  path: string;
  /**
   * Values by property key; computed cells may hold an inline error.
   */
  row: ComputedRow;
  /**
   * The row's title.
   */
  title: string;
}

/**
 * The result of a row write.
 */
export interface CollectionsUpdateRowRowWritten {
  /**
   * Every row file written or removed, including mirrored relations.
   */
  changed: Changeset;
  /**
   * The row after the write; absent after a delete.
   */
  row?: CollectionsUpdateRowRowDetail | null;
}

/**
 * A column of a view result.
 */
export interface Column {
  /**
   * Whether the value can be edited (stored, not computed).
   */
  editable: boolean;
  /**
   * Number display format, for numeric columns.
   */
  format: NumberFormat;
  /**
   * Property key.
   */
  key: string;
  /**
   * Display name.
   */
  name: string;
  /**
   * Options, for select and multi-select columns.
   */
  options: SelectOption[];
  /**
   * Property type name (`number`, `select`…).
   */
  type: string;
}

/**
 * A comment.
 */
export interface CommentOut {
  /**
   * Author handle.
   */
  author: string;
  /**
   * Body in Vellum Markdown; empty once deleted.
   */
  body: string;
  /**
   * Whether a model wrote it for the author.
   */
  by_model: boolean;
  /**
   * Posted.
   */
  created_at: string;
  /**
   * Deleted, kept as a marker for its replies.
   */
  deleted: boolean;
  /**
   * Last edited.
   */
  edited_at?: string | null;
  /**
   * Comment ID.
   */
  id: string;
  /**
   * Reactions: emoji and the handles that chose it.
   */
  reactions: ReactionOut[];
}

/**
 * A comment by ID.
 */
export interface CommentRef {
  /**
   * Comment ID.
   */
  comment_id: string;
}

/**
 * The result of a comment command.
 */
export interface CommentResult {
  /**
   * The thread after the change; `None` when it was removed.
   */
  thread?: ThreadOut | null;
}

/**
 * Completing a task.
 */
export interface CompleteInput {
  /**
   * Also complete its open subtasks.
   */
  cascade?: boolean;
  /**
   * The task ID.
   */
  task: string;
}

/**
 * The result of completing a task.
 */
export interface CompleteView {
  /**
   * For a repeating task: the date it moved to. It stays open.
   */
  next?: string | null;
  /**
   * The task after the change.
   */
  task: TaskView;
}

/**
 * Every property of one row, stored and computed, by key.
 */
export interface ComputedRow {
  /**
   * Values by property key.
   */
  cells: Record<string, CellValue>;
  /**
   * The row.
   */
  id: DocId;
}

/**
 * A page's content.
 */
export interface ContentView {
  /**
   * The text; absent for a file that is not UTF-8 text.
   */
  content?: string | null;
  /**
   * The content's version tag; send it back as `base_etag` to save.
   */
  etag: string;
  /**
   * The item.
   */
  item: NodeView;
}

/**
 * A block with its whole section: what "ask this space" and MCP quote.
 */
export interface Context {
  /**
   * The block asked for.
   */
  block: StoredBlock;
  /**
   * The citation of the block.
   */
  citation: string;
  /**
   * Every block of its section, in reading order.
   */
  section: StoredBlock[];
}

/**
 * `search.context` input.
 */
export interface ContextInput {
  /**
   * The block, `blk-xxxxxxxx`.
   */
  block_id: BlockId;
  /**
   * The document.
   */
  doc_id: DocId;
}

/**
 * A new folder.
 */
export interface CreateFolder {
  /**
   * Its name.
   */
  name: string;
  /**
   * The folder to create it in; the top of the space when absent.
   */
  parent_id?: ObjectId | null;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * Creates a new collection from a CSV file, inferring property types.
 */
export interface CreateFromCsvInput {
  /**
   * The CSV text; the first row is the header.
   */
  csv: string;
  /**
   * Infer the schema and report it, without writing.
   */
  dry_run?: boolean;
  /**
   * The drive folder to create the `Title.collection` folder in.
   */
  folder: string;
  /**
   * The collection's title.
   */
  title: string;
}

/**
 * A new task.
 */
export interface CreateInput {
  /**
   * The list to add to when the line names none; the Inbox otherwise.
   */
  list?: string | null;
  /**
   * A task ID to add a subtask under.
   */
  parent?: string | null;
  /**
   * The quick-add line: title plus phrases such as `tomorrow 3pm`,
   * `every weekday`, `#list`, `@label`, `+person`, `p1`.
   */
  text: string;
}

/**
 * A new issue.
 */
export interface CreateIssue {
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Description in Vellum Markdown.
   */
  description?: string;
  /**
   * Due date, `YYYY-MM-DD`.
   */
  due?: string | null;
  /**
   * Estimate (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Intake (form, email, Slack): goes to triage when the team uses it.
   */
  intake?: boolean;
  /**
   * Label references.
   */
  labels?: string[];
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0–4.
   */
  priority?: number | null;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * The selected text, for a block.
   */
  quote?: string | null;
  /**
   * `<doc id>#blk-…` when filed from a page.
   */
  source_block?: string | null;
  /**
   * Status slug; the team default (or triage, for intake) when unset.
   */
  state?: string | null;
  /**
   * Team key.
   */
  team: string;
  /**
   * Template name, applied first.
   */
  template?: string | null;
  /**
   * Title.
   */
  title: string;
}

/**
 * A new list.
 */
export interface CreateListInput {
  /**
   * Its title.
   */
  title: string;
}

/**
 * A new page.
 */
export interface CreatePage {
  /**
   * Its Markdown body.
   */
  body?: string;
  /**
   * The folder to create it in; the top of the space when absent.
   */
  parent_id?: ObjectId | null;
  /**
   * The space.
   */
  space_id: SpaceId;
  /**
   * Its title.
   */
  title: string;
}

/**
 * Creates a row.
 */
export interface CreateRowInput {
  /**
   * The row page's Markdown body.
   */
  body?: string;
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * Values by property key or name.
   */
  values?: Record<string, Value>;
}

/**
 * A new shortcut.
 */
export interface CreateShortcut {
  /**
   * The folder to put it in; the top of the space when absent.
   */
  parent_id?: ObjectId | null;
  /**
   * The space to put it in.
   */
  space_id: SpaceId;
  /**
   * The item it points at.
   */
  target_id: ObjectId;
}

/**
 * `books.create_snapshot`.
 */
export interface CreateSnapshot {
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * What it marks.
   */
  message?: string;
  /**
   * The tag: letters, digits, `.`, `-` and `_`.
   */
  tag: string;
}

/**
 * A new space.
 */
export interface CreateSpace {
  /**
   * Shared drive, or the caller's own My Drive.
   */
  kind: SpaceKindView;
  /**
   * The display name.
   */
  name: string;
  /**
   * The short name in its folder path: lowercase letters, digits and
   * dashes.
   */
  slug: string;
}

/**
 * Create a team.
 */
export interface CreateTeam {
  /**
   * Key prefix, two to seven characters, such as `ENG`.
   */
  key: string;
  /**
   * Lead handle.
   */
  lead?: string | null;
  /**
   * Member handles.
   */
  members?: string[];
  /**
   * Display name.
   */
  name: string;
}

/**
 * A new API token.
 */
export interface CreateToken {
  /**
   * Days until it expires; never when absent.
   */
  expiresInDays?: number | null;
  /**
   * A label, such as "CI" or "Claude Desktop".
   */
  name: string;
  /**
   * The scopes to grant, no more than your own.
   */
  scopes: Scope[];
}

/**
 * A user an administrator creates.
 */
export interface CreateUser {
  /**
   * Whether the user administers the workspace.
   */
  admin?: boolean;
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The display name.
   */
  name: string;
  /**
   * An initial password; absent for SSO-only accounts.
   */
  password?: string | null;
}

/**
 * The result of creating an issue.
 */
export interface CreatedIssue {
  /**
   * For an issue filed from a block: the chip the caller writes into
   * the document (the document belongs to the drive, not Work).
   */
  chip?: ChipOut | null;
  /**
   * The new issue.
   */
  issue: IssueOut;
}

/**
 * A created token: the secret is shown only once.
 */
export interface CreatedToken {
  /**
   * The token's details.
   */
  details: TokenView;
  /**
   * The token, starting `vlm_`. Store it now; it is not shown again.
   */
  token: string;
}

/**
 * A CSV export.
 */
export interface CsvExport {
  /**
   * The CSV text: a header row, then one line per row.
   */
  csv: string;
  /**
   * A suggested file name (`Requirements - Open.csv`).
   */
  fileName: string;
}

/**
 * What a CSV import did, or would do on a dry run.
 */
export interface CsvImported {
  /**
   * Row files written, including links mirrored into other collections.
   */
  changed: Changeset;
  /**
   * The collection after the import: its properties (inferred, for a
   * new collection) and views.
   */
  collection: CollectionsImportCsvCollectionInfo;
  /**
   * Problems with individual cells; their values were kept as text or
   * left empty.
   */
  diagnostics: Diagnostic[];
  /**
   * Rows created (or that would be).
   */
  rows: number;
  /**
   * Headers that matched no writable property and were ignored.
   */
  skippedColumns: string[];
  /**
   * Whether anything was written.
   */
  written: boolean;
}

/**
 * A cycle.
 */
export interface CycleOut {
  /**
   * Daily burn-up: date, scope points, completed points.
   */
  burn_up: BurnOut[];
  /**
   * Completed points recorded so far.
   */
  completed_points: number;
  /**
   * Day after the last.
   */
  ends_on: string;
  /**
   * Key, such as `2026-C44`.
   */
  key: string;
  /**
   * Display name.
   */
  name?: string | null;
  /**
   * First day.
   */
  starts_on: string;
}

/**
 * A team's cycles.
 */
export interface CyclesOut {
  /**
   * Cycles by start date.
   */
  cycles: CycleOut[];
  /**
   * Mean completed points over the last three cycles.
   */
  velocity?: number | null;
}

/**
 * Start and end dates of a calendar entry or timeline bar.
 */
export interface DateSpan {
  /**
   * End (equal to start for single-day entries).
   */
  end: DateValue;
  /**
   * Start.
   */
  start: DateValue;
}

/**
 * A UTC date, YYYY-MM-DD, optionally with THH:MM[:SS]
 */
export type DateValue = string;

/**
 * Deletes one object.
 */
export interface DeleteObject {
  /**
   * The version being deleted.
   */
  if_match: string;
  /**
   * Its path inside the space folder.
   */
  path: string;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * A deleted object.
 */
export interface Deleted {
  /**
   * Its path inside the space folder.
   */
  path: string;
}

/**
 * A removed task.
 */
export interface DeletedView {
  /**
   * Its ID.
   */
  id: string;
}

/**
 * One enrolled device.
 */
export interface DeviceKeyInfo {
  /**
   * When it was enrolled.
   */
  added_at: Timestamp;
  /**
   * The first 16 bytes of SHA-256 of the key, 32 hex digits, as stored
   * in envelope stanzas.
   */
  fingerprint: string;
  /**
   * The device's name.
   */
  label: string;
  /**
   * True for the caller's own devices.
   */
  mine: boolean;
  /**
   * The X25519 public key, 64 hex digits.
   */
  public_key: string;
  /**
   * The member the device belongs to.
   */
  user_id: UserId;
}

/**
 * A problem found while loading files or importing CSV.
 *
 * Loading never fails on a bad row: the row still loads, the offending
 * value is preserved verbatim, and a diagnostic says what happened.
 */
export interface Diagnostic {
  /**
   * The file or CSV line it refers to.
   */
  location: string;
  /**
   * What happened, for people.
   */
  message: string;
  /**
   * How serious it is.
   */
  severity: Severity;
}

/**
 * A section whose text changed.
 */
export interface DiffChanged {
  /**
   * Lines added.
   */
  added: number;
  /**
   * The section.
   */
  id: string;
  /**
   * Its label after the change.
   */
  label: string;
  /**
   * A unified diff of the file.
   */
  patch: string;
  /**
   * Lines removed.
   */
  removed: number;
}

/**
 * `books.diff`: what changed between a snapshot and another snapshot or
 * the current book.
 */
export interface DiffInput {
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * The older snapshot's tag.
   */
  from: string;
  /**
   * The newer snapshot's tag; the current book when absent.
   */
  to?: string | null;
}

/**
 * A section that moved, was renumbered or renamed.
 */
export interface DiffMoved {
  /**
   * Its label before.
   */
  from: string;
  /**
   * The section.
   */
  id: string;
  /**
   * Its label after.
   */
  to: string;
}

/**
 * What changed. Sections the caller cannot see are left out, and so are
 * the settings and summary changes unless the caller manages the book.
 */
export interface DiffOutput {
  /**
   * Sections only in the newer version.
   */
  added: DiffSection[];
  /**
   * Sections whose text changed.
   */
  changed: DiffChanged[];
  /**
   * Sections that moved, were renumbered or renamed.
   */
  moved: DiffMoved[];
  /**
   * Sections only in the older version.
   */
  removed: DiffSection[];
  /**
   * Book settings that changed.
   */
  settings: string[];
  /**
   * A unified diff of `SUMMARY.md`.
   */
  summary_patch: string;
}

/**
 * A section on one side of a diff.
 */
export interface DiffSection {
  /**
   * The section.
   */
  id: string;
  /**
   * Its label on that side.
   */
  label: string;
}

/**
 * The daily digest.
 */
export interface DigestView {
  /**
   * Vellum Markdown, or nothing when the day is clear.
   */
  markdown?: string | null;
}

/**
 * A DocId as a 26-character ULID.
 */
export type DocId = string;

/**
 * The kind of object a document is, for the drive's "type" filter.
 */
export type DocKind = "page" | "book" | "collection" | "row" | "issue" | "task_list" | "folder";

/**
 * An acknowledgement.
 */
export interface Done {
  /**
   * Always `true`.
   */
  ok: boolean;
}

/**
 * The outcome of a restore drill.
 */
export interface DrillReport {
  /**
   * The backup drilled.
   */
  backup: string;
  /**
   * Bytes verified.
   */
  bytes: number;
  /**
   * Restore time in milliseconds.
   */
  duration_ms: number;
  /**
   * Why the drill failed, when it did.
   */
  error?: string | null;
  /**
   * Files verified.
   */
  files: number;
  /**
   * Whether every file restored and matched the manifest within the RTO.
   */
  passed: boolean;
  /**
   * When the drill ran.
   */
  ran_at: Timestamp;
  /**
   * Whether the restore finished within [`RTO_SECONDS`].
   */
  within_rto: boolean;
}

/**
 * An item in a space.
 */
export interface DriveCopyNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An item in a space.
 */
export interface DriveCreateFolderNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An item in a space.
 */
export interface DriveCreatePageNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An item in a space.
 */
export interface DriveCreateShortcutNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * A space.
 */
export interface DriveCreateSpaceSpaceView {
  /**
   * The space.
   */
  id: SpaceId;
  /**
   * Shared drive or My Drive.
   */
  kind: SpaceKindView;
  /**
   * The display name.
   */
  name: string;
  /**
   * The owner of a My Drive.
   */
  owner?: UserId | null;
  /**
   * Quota, trash and version settings.
   */
  settings: SettingsView;
  /**
   * The short name in its folder path.
   */
  slug: string;
}

/**
 * An item in a space.
 */
export interface DriveGetNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An item in a space.
 */
export interface DriveMoveNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An operation that returns nothing else.
 */
export interface DrivePurgeDone {
  /**
   * Always true.
   */
  ok: boolean;
}

/**
 * An item in a space.
 */
export interface DriveRenameNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An item in a space.
 */
export interface DriveResolveShortcutNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An item in a space.
 */
export interface DriveRestoreNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * An operation that returns nothing else.
 */
export interface DriveStarDone {
  /**
   * Always true.
   */
  ok: boolean;
}

/**
 * An item in a space.
 */
export interface DriveTrashNodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * A space.
 */
export interface DriveUpdateSpaceSpaceView {
  /**
   * The space.
   */
  id: SpaceId;
  /**
   * Shared drive or My Drive.
   */
  kind: SpaceKindView;
  /**
   * The display name.
   */
  name: string;
  /**
   * The owner of a My Drive.
   */
  owner?: UserId | null;
  /**
   * Quota, trash and version settings.
   */
  settings: SettingsView;
  /**
   * The short name in its folder path.
   */
  slug: string;
}

/**
 * One enrolled device.
 */
export interface E2eEnrolDeviceKeyInfo {
  /**
   * When it was enrolled.
   */
  added_at: Timestamp;
  /**
   * The first 16 bytes of SHA-256 of the key, 32 hex digits, as stored
   * in envelope stanzas.
   */
  fingerprint: string;
  /**
   * The device's name.
   */
  label: string;
  /**
   * True for the caller's own devices.
   */
  mine: boolean;
  /**
   * The X25519 public key, 64 hex digits.
   */
  public_key: string;
  /**
   * The member the device belongs to.
   */
  user_id: UserId;
}

/**
 * Edit a comment.
 */
export interface EditComment {
  /**
   * New body.
   */
  body: string;
  /**
   * Comment ID.
   */
  comment_id: string;
}

/**
 * Resolves a `collection` block embedded in a page.
 */
export interface EmbedInput {
  /**
   * The block's body, as written in the page:
   * `id: <collection id>`, then optional `view`, `type`, `filter`,
   * `sort`, `group`, `hide`, `date`, `end` and `limit` lines.
   */
  fence: string;
}

/**
 * The current rows of an embedded view.
 */
export interface EmbedResult {
  /**
   * The collection.
   */
  collectionId: CollectionId;
  /**
   * The `name.collection` folder, for an "open" link.
   */
  path: string;
  /**
   * The view after the block's overrides.
   */
  result: CollectionsEmbedViewResult;
  /**
   * Its title, for the block's caption.
   */
  title: string;
}

/**
 * No input.
 */
export type Empty = Record<string, never>;

/**
 * Enrols the caller's device in a space.
 */
export interface EnrolDevice {
  /**
   * A name the members recognise, such as "Dana's laptop".
   */
  label: string;
  /**
   * The device's X25519 public key, 64 hex digits, as
   * `vellum private key` prints it.
   */
  public_key: string;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * Exports a view as CSV.
 */
export interface ExportCsvInput {
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * A saved view's name; the first view when absent. The export has
   * that view's columns, filters and order, and every matching row.
   */
  view?: string | null;
}

/**
 * One question on a form.
 */
export interface FieldOut {
  /**
   * The answer's key in a submission.
   */
  key: string;
  /**
   * `text`, `long_text`, `select` or `email`.
   */
  kind: string;
  /**
   * The question.
   */
  label: string;
  /**
   * Where the answer goes: `description`, `title`, `priority` or `label`.
   */
  maps_to: string;
  /**
   * The choices of a `select` question.
   */
  options: string[];
  /**
   * Whether an answer is needed.
   */
  required: boolean;
}

/**
 * A task's fields as strings, in the attribute-span syntax (D82).
 */
export interface FieldsView {
  /**
   * A time block: local start and minutes, `2026-11-03T15:00/90`.
   */
  block?: string | null;
  /**
   * When it was completed (UTC).
   */
  done_at?: string | null;
  /**
   * Deadline, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.
   */
  due?: string | null;
  /**
   * Labels.
   */
  labels: string[];
  /**
   * Priority 1 (highest) to 4.
   */
  priority?: number | null;
  /**
   * `completion` when the next date counts from completion.
   */
  recur_from?: string | null;
  /**
   * Reminders: ISO durations relative to the deadline, or local times.
   */
  remind: string[];
  /**
   * RFC 5545 repeat rule, without `RRULE:`.
   */
  rrule?: string | null;
  /**
   * Start date, `YYYY-MM-DD`.
   */
  start?: string | null;
  /**
   * `evening`, `anytime` or `someday`.
   */
  when?: string | null;
}

/**
 * Search filters; every filter given must match.
 */
export interface FindInput {
  /**
   * Who created the item.
   */
  created_by?: UserId | null;
  /**
   * Kinds to keep; all when empty.
   */
  kinds?: NodeKindView[];
  /**
   * At most this many, 1 to 200; 50 when absent.
   */
  limit?: number | null;
  /**
   * A space or folder the item must be in, at any depth.
   */
  location?: ObjectId | null;
  /**
   * Modified at or after this time.
   */
  modified_after?: Timestamp | null;
  /**
   * Modified before this time.
   */
  modified_before?: Timestamp | null;
  /**
   * Only items the caller starred.
   */
  starred?: boolean;
  /**
   * Case-insensitive text the title must contain.
   */
  title?: string | null;
}

/**
 * An issue in My Issues, with why it sits where it does.
 */
export interface FocusedIssue {
  /**
   * urgent, sla, blocked, cycle, active, triage, backlog or done.
   */
  focus: string;
  /**
   * The issue.
   */
  issue: IssueOut;
}

/**
 * An intake form.
 */
export interface FormOut {
  /**
   * Help shown above the questions.
   */
  description: string;
  /**
   * The questions, in order.
   */
  fields: FieldOut[];
  /**
   * Whether people outside the workspace may submit through a share link.
   */
  public: boolean;
  /**
   * The form's slug, unique in its team.
   */
  slug: string;
  /**
   * The form's title, such as "Request access".
   */
  title: string;
}

/**
 * A team's intake forms.
 */
export interface FormsOut {
  /**
   * The forms, in the order the team lists them.
   */
  forms: FormOut[];
}

/**
 * A run of snippet text, highlighted when it matched the query.
 */
export interface Fragment {
  /**
   * Whether it matched a query term.
   */
  hit: boolean;
  /**
   * The text.
   */
  text: string;
}

/**
 * Downloads one object.
 */
export interface GetObject {
  /**
   * Its path inside the space folder.
   */
  path: string;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * Whether Google Calendar is set up and connected.
 */
export interface GoogleView {
  /**
   * Whether the server has a Google `OAuth` client.
   */
  configured: boolean;
  /**
   * Whether the person has connected their calendar.
   */
  connected: boolean;
}

/**
 * A group of rows (one board column or table section).
 */
export interface Group {
  /**
   * The heading to show.
   */
  label: string;
  /**
   * The rows.
   */
  rows: RowView[];
  /**
   * The group's value; `None` for the "no value" group or an ungrouped view.
   */
  value?: Value | null;
}

/**
 * One group of issues.
 */
export interface GroupOut {
  /**
   * The issues, in order.
   */
  issues: IssueOut[];
  /**
   * The group's value (`in-review`, `urgent`, `dev`); `None` is "none".
   */
  key?: string | null;
}

/**
 * A phrase to highlight in the quick-add box.
 */
export interface HighlightView {
  /**
   * End, exclusive, in UTF-16 code units.
   */
  end: number;
  /**
   * What it set.
   */
  kind: TasksParseTokenView;
  /**
   * Start, in UTF-16 code units (`JavaScript` string indices).
   */
  start: number;
}

/**
 * A user as the API shows it.
 */
export interface IdentityCreateUserUserView {
  /**
   * Whether the user administers the workspace.
   */
  admin: boolean;
  /**
   * When the account was created.
   */
  createdAt: Timestamp;
  /**
   * Whether sign-in is blocked.
   */
  disabled: boolean;
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The unique handle used for assignees and mentions.
   */
  handle: string;
  /**
   * Whether the account has a password (otherwise SSO only).
   */
  hasPassword: boolean;
  /**
   * The user's ID.
   */
  id: UserId;
  /**
   * The display name.
   */
  name: string;
}

/**
 * A user as the API shows it.
 */
export interface IdentitySetupUserView {
  /**
   * Whether the user administers the workspace.
   */
  admin: boolean;
  /**
   * When the account was created.
   */
  createdAt: Timestamp;
  /**
   * Whether sign-in is blocked.
   */
  disabled: boolean;
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The unique handle used for assignees and mentions.
   */
  handle: string;
  /**
   * Whether the account has a password (otherwise SSO only).
   */
  hasPassword: boolean;
  /**
   * The user's ID.
   */
  id: UserId;
  /**
   * The display name.
   */
  name: string;
}

/**
 * A user as the API shows it.
 */
export interface IdentityUpdateMeUserView {
  /**
   * Whether the user administers the workspace.
   */
  admin: boolean;
  /**
   * When the account was created.
   */
  createdAt: Timestamp;
  /**
   * Whether sign-in is blocked.
   */
  disabled: boolean;
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The unique handle used for assignees and mentions.
   */
  handle: string;
  /**
   * Whether the account has a password (otherwise SSO only).
   */
  hasPassword: boolean;
  /**
   * The user's ID.
   */
  id: UserId;
  /**
   * The display name.
   */
  name: string;
}

/**
 * A user as the API shows it.
 */
export interface IdentityUpdateUserUserView {
  /**
   * Whether the user administers the workspace.
   */
  admin: boolean;
  /**
   * When the account was created.
   */
  createdAt: Timestamp;
  /**
   * Whether sign-in is blocked.
   */
  disabled: boolean;
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The unique handle used for assignees and mentions.
   */
  handle: string;
  /**
   * Whether the account has a password (otherwise SSO only).
   */
  hasPassword: boolean;
  /**
   * The user's ID.
   */
  id: UserId;
  /**
   * The display name.
   */
  name: string;
}

/**
 * Adds the rows of a CSV file to an existing collection.
 */
export interface ImportCsvInput {
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * The CSV text. Headers match property names or keys; relation cells
   * match related rows by title.
   */
  csv: string;
  /**
   * Check the file and report what would happen, without writing.
   */
  dry_run?: boolean;
}

/**
 * Which app an export comes from.
 */
export type ImportFrom = "google_tasks" | "todoist" | "things";

/**
 * An export to import.
 */
export interface ImportInput {
  /**
   * The export's text.
   */
  content: string;
  /**
   * The app.
   */
  from: ImportFrom;
  /**
   * The list title for a Todoist CSV, which does not name its project.
   */
  project?: string | null;
}

/**
 * The result of an import.
 */
export interface ImportView {
  /**
   * The lists written.
   */
  lists: ImportedList[];
}

/**
 * One list written by an import.
 */
export interface ImportedList {
  /**
   * How many items were added; items already imported are skipped.
   */
  added: number;
  /**
   * The list's document.
   */
  doc: DocId;
  /**
   * Its title.
   */
  title: string;
}

/**
 * An inbox line, as the API shows it.
 */
export interface InboxItem {
  /**
   * Who caused it.
   */
  actor?: string | null;
  /**
   * When it happened (RFC 3339).
   */
  at: string;
  /**
   * Short body text.
   */
  body: string;
  /**
   * Pass it to `notify.read` to mark the line read.
   */
  id: string;
  /**
   * The kind, such as `mentioned`.
   */
  kind: string;
  /**
   * Whether it was read.
   */
  read: boolean;
  /**
   * What it is about, such as `ENG-142`.
   */
  subject: string;
  /**
   * One line.
   */
  title: string;
  /**
   * Link into the app.
   */
  url: string;
}

/**
 * Your inbox.
 */
export interface InboxPage {
  /**
   * Lines, newest first.
   */
  items: InboxItem[];
  /**
   * How many are unread.
   */
  unread: number;
}

/**
 * Which inbox lines to list.
 */
export interface InboxQuery {
  /**
   * Only unread lines.
   */
  unread_only?: boolean;
}

/**
 * What the browser reported, joined to its server trace.
 */
export interface IngestedOutput {
  /**
   * Whether the server trace was still in the buffer.
   */
  joined: boolean;
  /**
   * Spans added.
   */
  spans: number;
}

/**
 * The answer to an insights question.
 */
export interface InsightsOut {
  /**
   * Per breakdown value, largest first.
   */
  buckets: BucketOut[];
  /**
   * Everything together.
   */
  overall: StatsOut;
}

/**
 * An insights question.
 */
export interface InsightsQuery {
  /**
   * A filter as JSON, the same as a view's; empty means all issues.
   */
  filter?: string | null;
  /**
   * First day of the range, `YYYY-MM-DD`.
   */
  from: string;
  /**
   * Split each breakdown into `week`s or `month`s.
   */
  interval?: string | null;
  /**
   * `lead_time` (default), `cycle_time`, `triage_time`, `throughput`
   * or `created`.
   */
  measure?: string | null;
  /**
   * Break down by `none` (default), `team`, `assignee`, `priority`,
   * `label`, `project`, `cycle` or `estimate`.
   */
  pivot?: string | null;
  /**
   * Only this team's issues (its key).
   */
  team?: string | null;
  /**
   * Day after the last day of the range, `YYYY-MM-DD`.
   */
  to: string;
}

/**
 * A list of issues in groups.
 */
export interface IssueGroups {
  /**
   * Groups in display order.
   */
  groups: GroupOut[];
}

/**
 * An issue.
 */
export interface IssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * An issue by identifier.
 */
export interface IssueRef {
  /**
   * `ENG-142`, an old alias, or the issue ID.
   */
  key: string;
}

/**
 * How many results to return.
 */
export interface Limit {
  /**
   * At most this many, 1 to 200; 50 when absent.
   */
  limit?: number | null;
}

/**
 * A link on an issue.
 */
export interface LinkOut {
  /**
   * `source`, `doc`, `pull_request` or another kind.
   */
  kind?: string | null;
  /**
   * Title.
   */
  title?: string | null;
  /**
   * URL; `vellum://doc/<id>#blk-…` for document blocks.
   */
  url: string;
}

/**
 * `books.list` takes no input.
 */
export type ListBooks = Record<string, unknown>;

/**
 * The items in a folder, or at the top of a space.
 */
export interface ListChildren {
  /**
   * The folder; the top of the space when absent.
   */
  parent_id?: ObjectId | null;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * `collections.list` takes no input.
 */
export type ListCollections = Record<string, never>;

/**
 * Threads on a target.
 */
export interface ListComments {
  /**
   * An issue key, a document ID, or `<doc id>#blk-…`.
   */
  target: string;
}

/**
 * Lists a space's objects, a page at a time, in path order.
 */
export interface ListObjects {
  /**
   * Start after this path (the previous page's `next`).
   */
  after?: string | null;
  /**
   * At most this many (default and maximum 1000).
   */
  limit?: number | null;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * A task list.
 */
export interface ListView {
  /**
   * The list's document.
   */
  doc: DocId;
  /**
   * Whether it is the person's Inbox.
   */
  inbox: boolean;
  /**
   * Members' handles.
   */
  members: string[];
  /**
   * How many open items it holds.
   */
  open: number;
  /**
   * The owner's handle.
   */
  owner?: string | null;
  /**
   * The title.
   */
  title: string;
}

/**
 * The person's task lists.
 */
export interface ListsView {
  /**
   * Inbox first, then by title.
   */
  lists: ListView[];
}

/**
 * Which lines to mark read.
 */
export interface MarkRead {
  /**
   * Every line instead.
   */
  all?: boolean;
  /**
   * Line IDs.
   */
  ids?: string[];
}

/**
 * Sharing a list.
 */
export interface MemberInput {
  /**
   * The list's document.
   */
  list_id: DocId;
  /**
   * The person's handle, with or without `@`.
   */
  member: string;
}

/**
 * A mention by the ID of the comment that made it.
 */
export interface MentionRef {
  /**
   * The comment ID.
   */
  comment: string;
}

/**
 * Dragging a task to another bucket.
 */
export interface MoveInput {
  /**
   * The task ID.
   */
  task: string;
  /**
   * `inbox`, `today`, `evening`, `anytime`, `someday` or a date
   * (`YYYY-MM-DD`). Moves the start date; the deadline stays.
   */
  to: string;
}

/**
 * Move an issue to another team.
 */
export interface MoveIssue {
  /**
   * The issue.
   */
  key: string;
  /**
   * The new team's key.
   */
  to: string;
}

/**
 * `books.move_section`.
 */
export interface MoveSection {
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * Where it goes.
   */
  place: BooksMoveSectionPlaceInput;
  /**
   * The section to move, with everything under it.
   */
  section_id: string;
}

/**
 * The caller's My Drive, if they have one.
 */
export interface MyDrive {
  /**
   * The space. It is made on the first visit when the account has
   * none yet, so it is always present.
   */
  space?: SpaceView | null;
}

/**
 * My Issues.
 */
export interface MyIssues {
  /**
   * assigned (default), created or subscribed.
   */
  tab?: string | null;
}

/**
 * My Issues.
 */
export interface MyIssuesOut {
  /**
   * Issues in focus order.
   */
  issues: FocusedIssue[];
}

/**
 * My Work settings as a client sends them, and as they are stored.
 */
export interface MyWorkSettingsInput {
  /**
   * When the day starts, `HH:MM`; reminders on a date-only deadline
   * count from it.
   */
  day_starts_at?: string;
  /**
   * When the morning digest is sent, `HH:MM`; `null` turns it off.
   */
  digest_at?: string | null;
  /**
   * An IANA time zone such as `Europe/Berlin`; `null` follows the
   * server's zone.
   */
  time_zone?: string | null;
}

/**
 * A person's My Work settings.
 */
export interface MyWorkSettingsView {
  /**
   * When their day starts, `HH:MM`.
   */
  day_starts_at: string;
  /**
   * When their digest is sent, `HH:MM`; `null` when it is off.
   */
  digest_at?: string | null;
  /**
   * The zone My Work counts in for them now.
   */
  effective_time_zone: string;
  /**
   * The zone they chose; `null` when they follow the server's.
   */
  time_zone?: string | null;
}

/**
 * My Work for the signed-in person.
 */
export interface MyWorkView {
  /**
   * How many open tasks the view shows.
   */
  open: number;
  /**
   * The sections in order; empty ones are left out.
   */
  sections: SectionView[];
  /**
   * The person's local date.
   */
  today: string;
}

/**
 * An event to append.
 */
export interface NewAuditEvent {
  /**
   * What they did, as `<service>.<action>`, such as `admin.settings_update`.
   */
  action: string;
  /**
   * Who acted.
   */
  actor: AuditActor;
  /**
   * Action-specific details; never document content or secrets.
   */
  details: Record<string, unknown>;
  /**
   * How it ended.
   */
  outcome: Outcome;
  /**
   * The request id shown to the user.
   */
  request_id?: string | null;
  /**
   * What they acted on, such as a document or setting id.
   */
  target?: string | null;
  /**
   * The W3C trace id, to open the request in Admin → Requests.
   */
  trace_id?: string | null;
}

/**
 * No input.
 */
export type NoInput = Record<string, never>;

/**
 * Names an item (a folder, page, book, collection or shortcut).
 */
export interface NodeKey {
  /**
   * The item.
   */
  id: ObjectId;
}

/**
 * What an item is.
 */
export type NodeKindView = "folder" | "page" | "book" | "collection" | "shortcut";

/**
 * Items.
 */
export interface NodeList {
  /**
   * The items.
   */
  items: NodeView[];
}

/**
 * An item in a space.
 */
export interface NodeView {
  /**
   * Who created it, when known.
   */
  created_by?: UserId | null;
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * What it is.
   */
  kind: NodeKindView;
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * The folder it is in; absent at the top of the space.
   */
  parent_id?: ObjectId | null;
  /**
   * Its path in the workspace folder.
   */
  path: string;
  /**
   * Bytes on disk.
   */
  size: number;
  /**
   * The space it is in.
   */
  space_id: SpaceId;
  /**
   * What a shortcut points at.
   */
  target_id?: ObjectId | null;
  /**
   * Its title.
   */
  title: string;
  /**
   * When it was trashed; absent for a live item.
   */
  trashed_at?: Timestamp | null;
}

/**
 * No input.
 */
export type NotifyPreferencesEmpty = Record<string, unknown>;

/**
 * No input.
 */
export type NotifyPushKeyEmpty = Record<string, unknown>;

/**
 * How a numeric result is displayed.
 */
export type NumberFormat = "plain" | "percent";

/**
 * A ObjectId as a 26-character ULID.
 */
export type ObjectId = string;

/**
 * One object in a space.
 */
export interface ObjectInfo {
  /**
   * When it last changed.
   */
  modified: Timestamp;
  /**
   * Its path inside the space folder.
   */
  path: string;
  /**
   * Its stored size in bytes.
   */
  size: number;
}

/**
 * A page of objects.
 */
export interface ObjectList {
  /**
   * Pass as `after` for the next page; absent on the last one.
   */
  next?: string | null;
  /**
   * The objects, in path order.
   */
  objects: ObjectInfo[];
}

/**
 * The built-in observability tools.
 */
export interface ObservabilitySettings {
  /**
   * The p95 latency budget of every route (R-OB-4).
   */
  budgets?: Budgets;
  /**
   * Requests slower than this are highlighted, in milliseconds.
   */
  slow_request_ms?: number;
  /**
   * Traces kept for Admin → Requests.
   */
  trace_buffer_capacity?: number;
  /**
   * Memory budget for those traces, in `MiB`.
   */
  trace_buffer_mib?: number;
}

/**
 * How an audited action ended.
 */
export type Outcome = "success" | "denied" | "failure";

/**
 * One heading of the page outline.
 */
export interface OutlineEntry {
  /**
   * The element id, for `#fragment` links.
   */
  id: string;
  /**
   * 1 to 6.
   */
  level: number;
  /**
   * The heading text.
   */
  text: string;
}

/**
 * Names a page.
 */
export interface PageKey {
  /**
   * The page.
   */
  id: ObjectId;
}

/**
 * A quick-add line to parse.
 */
export interface ParseInput {
  /**
   * The line as typed.
   */
  text: string;
}

/**
 * A parsed quick-add line.
 */
export interface ParseView {
  /**
   * People named with `+person`.
   */
  assignees: string[];
  /**
   * The fields the phrases set.
   */
  fields: FieldsView;
  /**
   * Phrases to highlight.
   */
  highlights: HighlightView[];
  /**
   * The list named with `#list`.
   */
  list?: string | null;
  /**
   * The title without the recognised phrases.
   */
  title: string;
}

/**
 * Where to move or copy an item.
 */
export interface PlaceInput {
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * Where it goes among its new siblings, counted without it: 0 is
   * first. When absent the order is left as it was.
   */
  index?: number | null;
  /**
   * The folder or page it goes into; the top of the space when absent.
   */
  parent_id?: ObjectId | null;
  /**
   * The space it goes to.
   */
  space_id: SpaceId;
}

/**
 * One week or month of a series.
 */
export interface PointOut {
  /**
   * First day of the week or month, `YYYY-MM-DD`.
   */
  start: string;
  /**
   * The numbers for that interval.
   */
  stats: StatsOut;
}

/**
 * Notification settings, as the API shows them.
 */
export interface PreferencesView {
  /**
   * Channels (`in_app`, `email`, `push`) per kind. Every kind is
   * listed; on input, a kind left out keeps its defaults.
   */
  channels: Record<string, string[]>;
  /**
   * Minutes email waits for the inbox entry to be read (0 to 1440).
   */
  email_delay_minutes: number;
  /**
   * Issue keys and document IDs you muted.
   */
  muted: string[];
  /**
   * Non-urgent push and email are held until this time (RFC 3339).
   */
  snoozed_until?: string | null;
}

/**
 * What to profile.
 */
export interface ProfileInput {
  /**
   * Only this route template.
   */
  route?: string | null;
  /**
   * How far back to look, in seconds (default 60).
   */
  seconds?: number | null;
}

/**
 * The profile endpoint's response.
 */
export interface ProfileResponse {
  /**
   * Folded stacks, one `frame;frame;frame microseconds` line each.
   */
  folded: string;
  /**
   * Traces included.
   */
  traces: number;
}

/**
 * A project with its computed progress.
 */
export interface ProjectOut {
  /**
   * Project ID.
   */
  id: string;
  /**
   * Issues in the project.
   */
  issues: number;
  /**
   * Lead handle.
   */
  lead?: string | null;
  /**
   * The project file, front matter and spec.
   */
  markdown: string;
  /**
   * Percent complete by points.
   */
  percent: number;
  /**
   * planned, started, paused, completed or canceled.
   */
  status: string;
  /**
   * Target date.
   */
  target_date?: string | null;
  /**
   * Team keys.
   */
  teams: string[];
  /**
   * Title.
   */
  title: string;
}

/**
 * A project by ID.
 */
export interface ProjectRef {
  /**
   * Project ID.
   */
  project_id: string;
}

/**
 * Every project.
 */
export interface ProjectsOut {
  /**
   * Projects.
   */
  projects: ProjectOut[];
}

/**
 * One property, as the UI needs it to render headers and editors.
 */
export interface PropertyInfo {
  /**
   * Whether the value is computed (and read-only).
   */
  computed: boolean;
  /**
   * Number display format, where it applies.
   */
  format?: NumberFormat | null;
  /**
   * For a formula: its source text.
   */
  formula?: string | null;
  /**
   * Stable key used in row front matter.
   */
  key: string;
  /**
   * Display name.
   */
  name: string;
  /**
   * Options of a select or multi-select.
   */
  options: SelectOption[];
  /**
   * For a relation.
   */
  relation?: RelationInfo | null;
  /**
   * For a rollup.
   */
  rollup?: RollupInfo | null;
  /**
   * The type name (`title`, `select`, `formula`, …).
   */
  type: string;
}

/**
 * The result of a push registration change.
 */
export interface PushDevices {
  /**
   * How many browsers you have registered now.
   */
  devices: number;
}

/**
 * The VAPID public key.
 */
export interface PushKey {
  /**
   * Base64url, uncompressed P-256; pass it as `applicationServerKey`.
   */
  publicKey: string;
}

/**
 * The browser's keys.
 */
export interface PushKeys {
  /**
   * The 16-byte auth secret, base64url.
   */
  auth: string;
  /**
   * The browser's P-256 public key, base64url.
   */
  p256dh: string;
}

/**
 * Uploads a page sealed on the device.
 */
export interface PutObject {
  /**
   * The recipient envelope, base64. It must be sealed to exactly the
   * space's current keys.
   */
  envelope: string;
  /**
   * The version this write replaces; absent to create a new object.
   */
  if_match?: string | null;
  /**
   * Its path inside the space folder.
   */
  path: string;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * Create or replace a project.
 */
export interface PutProject {
  /**
   * The whole `project.md`.
   */
  markdown: string;
  /**
   * Project ID; must match the file.
   */
  project_id: string;
}

/**
 * Replace a team's configuration.
 */
export interface PutTeam {
  /**
   * Team key; must match the file.
   */
  team: string;
  /**
   * The whole `team.yaml`.
   */
  yaml: string;
}

/**
 * Runs a saved view, with optional ad-hoc changes on top.
 */
export interface QueryInput {
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * Extra filters, added to the view's (`Status = Done`).
   */
  filter?: string[];
  /**
   * Group-by property, replacing the view's.
   */
  group?: string | null;
  /**
   * Hidden properties, replacing the view's.
   */
  hide?: string[];
  /**
   * Another layout for this query only.
   */
  kind?: ViewKind | null;
  /**
   * Most rows to return.
   */
  limit?: number | null;
  /**
   * Sort keys replacing the view's (`Due desc`).
   */
  sort?: string[];
  /**
   * A saved view's name; the first view when absent.
   */
  view?: string | null;
}

/**
 * Which issues to list, and how.
 */
export interface QueryIssues {
  /**
   * A Work filter as JSON text, such as
   * `{"field":"assignee","op":"is","values":["@me"]}`.
   */
  filter?: string | null;
  /**
   * status, assignee, priority, project, cycle, label, team or none.
   */
  group_by?: string | null;
  /**
   * list or board.
   */
  layout?: string | null;
  /**
   * At most this many issues (default 250, at most 1000).
   */
  limit?: number | null;
  /**
   * priority, updated, created, due, estimate or title.
   */
  order_by?: string | null;
  /**
   * `all`, `past_week`, `past_month` or `none`.
   */
  show_closed?: string | null;
  /**
   * Limit to one team.
   */
  team?: string | null;
}

/**
 * Add or remove a reaction.
 */
export interface React {
  /**
   * Comment ID.
   */
  comment_id: string;
  /**
   * The emoji.
   */
  emoji: string;
  /**
   * Add (`true`, the default) or remove.
   */
  on?: boolean;
}

/**
 * People who reacted with one emoji.
 */
export interface ReactionOut {
  /**
   * Handles.
   */
  by: string[];
  /**
   * The emoji.
   */
  emoji: string;
}

/**
 * A recently opened item.
 */
export interface RecentItem {
  /**
   * The item.
   */
  item: NodeView;
  /**
   * When the caller last opened it.
   */
  opened_at: Timestamp;
}

/**
 * The caller's recently opened items, newest first.
 */
export interface RecentList {
  /**
   * The items.
   */
  items: RecentItem[];
}

/**
 * A region of the summary.
 */
export type Region = "prefix" | "numbered" | "suffix";

/**
 * A relation to add or remove.
 */
export interface RelationIn {
  /**
   * The other issue's key.
   */
  issue: string;
  /**
   * `blocks`, `blocked_by`, `related` or `duplicate_of`.
   */
  kind: string;
}

/**
 * A relation's target.
 */
export interface RelationInfo {
  /**
   * The paired property there, for a two-way relation.
   */
  back?: string | null;
  /**
   * The related collection.
   */
  to: CollectionId;
}

/**
 * A relation from one issue to another.
 */
export interface RelationOut {
  /**
   * The other issue's key.
   */
  issue: string;
  /**
   * `blocks`, `blocked_by`, `related` or `duplicate_of`.
   */
  kind: string;
}

/**
 * A count of what an operation removed.
 */
export interface Removed {
  /**
   * How many items or versions were removed.
   */
  removed: number;
}

/**
 * A new title.
 */
export interface RenameInput {
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * Its new title.
   */
  title: string;
}

/**
 * Something on the page that rendered with a fallback.
 */
export interface RenderWarning {
  /**
   * The top-level block it came from, from 0.
   */
  block: number;
  /**
   * What happened, for the author.
   */
  message: string;
}

/**
 * A page rendered for the reader.
 */
export interface RenderedPageView {
  /**
   * A strong `ETag` of the HTML (`sha256-…`).
   */
  etag: string;
  /**
   * Sanitised HTML: one `<article class="vl-doc">`, styled by the theme
   * stylesheet's `--vl-*` variables.
   */
  html: string;
  /**
   * The page.
   */
  id: ObjectId;
  /**
   * The page outline.
   */
  outline: OutlineEntry[];
  /**
   * The renderer version that produced it.
   */
  render_version: number;
  /**
   * Diagrams shown from their last good picture because the diagram
   * server was unavailable.
   */
  stale_figures: number;
  /**
   * The front matter title, or the first level-1 heading.
   */
  title?: string | null;
  /**
   * Blocks that rendered with a fallback.
   */
  warnings: RenderWarning[];
}

/**
 * Filters for the request list.
 */
export interface RequestsInput {
  /**
   * Only requests older than this cursor.
   */
  before?: number | null;
  /**
   * Only failed requests.
   */
  errors_only?: boolean;
  /**
   * A request id or trace id.
   */
  id?: string | null;
  /**
   * At most this many rows (up to 500).
   */
  limit?: number | null;
  /**
   * Only requests at least this long, in milliseconds.
   */
  min_duration_ms?: number | null;
  /**
   * Only this route template.
   */
  route?: string | null;
}

/**
 * The request list, newest first.
 */
export interface RequestsOutput {
  /**
   * One summary per request.
   */
  requests: unknown[];
}

/**
 * Resolve or reopen a thread.
 */
export interface ResolveThread {
  /**
   * Resolve (`true`, the default) or reopen.
   */
  resolved?: boolean;
  /**
   * Thread ID.
   */
  thread_id: string;
}

/**
 * Revokes one device key.
 */
export interface RevokeDevice {
  /**
   * The key's fingerprint, 32 hex digits.
   */
  fingerprint: string;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * What a revocation removed. Members' devices strip these keys from
 * every envelope in the space (`vellum private reseal`).
 */
export interface Revoked {
  /**
   * The removed keys.
   */
  removed: DeviceKeyInfo[];
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * How a rollup is computed.
 */
export interface RollupInfo {
  /**
   * How values are combined.
   */
  agg: Aggregation;
  /**
   * The property read on related rows.
   */
  prop: string;
  /**
   * The relation property followed.
   */
  via: string;
}

/**
 * A budget for one route, or every route under a prefix.
 */
export interface RouteBudget {
  /**
   * The HTTP method, or every method when absent.
   */
  method?: string | null;
  /**
   * The p95 budget in milliseconds.
   */
  p95_ms: number;
  /**
   * A route template such as `/v1/pages/{id}`, or a prefix ending in
   * `*` such as `/v1/export/*`.
   */
  route: string;
}

/**
 * Live RED and saturation figures for the console.
 */
export interface RoutesView {
  /**
   * The ring buffer's counters.
   */
  buffer: unknown;
  /**
   * Requests in progress.
   */
  in_flight: number;
  /**
   * The window in minutes.
   */
  minutes: number;
  /**
   * Process memory and CPU.
   */
  process: unknown;
  /**
   * Per-route figures, busiest first.
   */
  routes: unknown[];
}

/**
 * One row with every value, stored and computed.
 */
export interface RowDetail {
  /**
   * The page's Markdown body.
   */
  body: string;
  /**
   * The collection.
   */
  collectionId: CollectionId;
  /**
   * The row page's path.
   */
  path: string;
  /**
   * Values by property key; computed cells may hold an inline error.
   */
  row: ComputedRow;
  /**
   * The row's title.
   */
  title: string;
}

/**
 * Names one row.
 */
export interface RowKey {
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * The row (the `id` in its front matter).
   */
  row_id: DocId;
}

/**
 * A row in a specific collection.
 */
export interface RowRef {
  /**
   * The collection.
   */
  collection: CollectionId;
  /**
   * The row.
   */
  row: DocId;
}

/**
 * One row of a view result.
 */
export interface RowView {
  /**
   * Cells, aligned with [`ViewResult::columns`].
   */
  cells: CellView[];
  /**
   * The row page.
   */
  id: DocId;
  /**
   * Dates, for calendar and timeline views.
   */
  span?: DateSpan | null;
  /**
   * The title text.
   */
  title: string;
}

/**
 * The result of a row write.
 */
export interface RowWritten {
  /**
   * Every row file written or removed, including mirrored relations.
   */
  changed: Changeset;
  /**
   * The row after the write; absent after a delete.
   */
  row?: CollectionsCreateRowRowDetail | null;
}

/**
 * A new version of a page's content.
 */
export interface SaveInput {
  /**
   * The content the edit started from, as read.
   */
  base_content: string;
  /**
   * The etag that came with `base_content`.
   */
  base_etag: string;
  /**
   * The edited content.
   */
  content: string;
  /**
   * The page.
   */
  id: ObjectId;
}

/**
 * How a save landed.
 */
export type SaveResult = "written" | "merged" | "conflicted";

/**
 * The outcome of a save.
 */
export interface SaveView {
  /**
   * The conflict copy holding the new content, after a conflict.
   */
  conflict_copy?: NodeView | null;
  /**
   * The page after the save.
   */
  item: NodeView;
  /**
   * How the save landed.
   */
  result: SaveResult;
  /**
   * The version written; absent after a conflict.
   */
  version_id?: VersionId | null;
}

/**
 * Saves a view: replaces the view with this name, or adds it at the end.
 * Filters, sorts and property names use the same text as `schema.yaml`
 * (`Status is Done`, `Due desc`), by property name or key.
 */
export interface SaveViewInput {
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * Date property for calendar and timeline.
   */
  date?: string | null;
  /**
   * End-date property for timeline bars.
   */
  end?: string | null;
  /**
   * Filters, all of which must match.
   */
  filter?: string[];
  /**
   * Group-by property (board columns, table sections).
   */
  group?: string | null;
  /**
   * Hidden properties.
   */
  hide?: string[];
  /**
   * The layout; a table when absent.
   */
  kind?: ViewKind | null;
  /**
   * Most rows to show.
   */
  limit?: number | null;
  /**
   * A new name, to rename the view.
   */
  rename_to?: string | null;
  /**
   * Sort keys, most significant first.
   */
  sort?: string[];
  /**
   * The view's name.
   */
  view: string;
}

/**
 * A permission ceiling carried by a credential.
 *
 * The set matches the MCP scopes in the brief: `read`, `write:draft`,
 * `write:direct`, `work` and `admin`.
 */
export type Scope = "read" | "write:draft" | "write:direct" | "work" | "admin";

/**
 * An object's ciphertext.
 */
export interface SealedObject {
  /**
   * The recipient envelope (or the space's folder manifest), base64.
   */
  envelope: string;
  /**
   * Its version, for `if_match` on the next write.
   */
  etag: string;
  /**
   * Its path inside the space folder.
   */
  path: string;
}

/**
 * One block that matched, with what the UI and MCP need to show and cite it.
 */
export interface SearchHit {
  /**
   * The block.
   */
  block: BlockId;
  /**
   * The block's kind.
   */
  block_kind: BlockKind;
  /**
   * A stable citation: `vellum://doc/{doc}#{block}`.
   */
  citation: string;
  /**
   * The document.
   */
  doc: DocId;
  /**
   * The document's kind.
   */
  doc_kind: DocKind;
  /**
   * Headings above the block.
   */
  heading_path: string[];
  /**
   * The document's path.
   */
  path: string;
  /**
   * The fused relevance score; higher is better. Only comparable within one response.
   */
  score: number;
  /**
   * The heading that opens the block's section.
   */
  section?: BlockId | null;
  /**
   * A short excerpt of the block with the matches marked.
   */
  snippet: Fragment[];
  /**
   * The space.
   */
  space: SpaceId;
  /**
   * The document's title.
   */
  title: string;
}

/**
 * One page of search results.
 */
export interface SearchPage {
  /**
   * Whether more hits follow this page.
   */
  has_more: boolean;
  /**
   * The hits, best first.
   */
  hits: SearchHit[];
  /**
   * Whether semantic ranking took part.
   */
  hybrid: boolean;
}

/**
 * `search.query` input.
 */
export interface SearchQueryQueryInput {
  /**
   * Comma-separated kinds: `page`, `book`, `collection`, `row`, `issue`,
   * `task_list`, `folder`.
   */
  kinds?: string | null;
  /**
   * Hits to return (default 20, at most 100).
   */
  limit?: number | null;
  /**
   * A folder, drive, book or space ID: only documents under it.
   */
  location?: ObjectId | null;
  /**
   * Only documents modified at or after this time (Unix milliseconds).
   */
  modified_from?: number | null;
  /**
   * Only documents modified at or before this time (Unix milliseconds).
   */
  modified_to?: number | null;
  /**
   * Hits to skip.
   */
  offset?: number;
  /**
   * Comma-separated owner (user) IDs.
   */
  owners?: string | null;
  /**
   * What to search for.
   */
  q: string;
  /**
   * Comma-separated space IDs to search in.
   */
  spaces?: string | null;
  /**
   * Only starred documents.
   */
  starred?: boolean;
  /**
   * Search the trash.
   */
  trashed?: boolean;
}

/**
 * A newly issued secret URL or address. It is shown once.
 */
export interface SecretView {
  /**
   * The feed URL or capture address.
   */
  value: string;
}

/**
 * Who may see, own and review a section.
 */
export interface SectionAccessInfo {
  /**
   * Its own `restrict_to`, as `book.yaml` stores it.
   */
  own_restrict_to?: string[] | null;
  /**
   * Effective owners.
   */
  owners: string[];
  /**
   * Who may see it (`user:<id>`, `group:<id>`), when it is restricted:
   * the intersection of its own and every enclosing restriction.
   */
  restrict_to?: string[] | null;
  /**
   * The sections whose restrictions apply, outermost first.
   */
  restricted_by: string[];
  /**
   * Effective reviewers.
   */
  reviewers: string[];
}

/**
 * One section.
 */
export interface SectionDoc {
  /**
   * Its access settings; only for book managers and owners.
   */
  access?: SectionAccessInfo | null;
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * The section.
   */
  id: string;
  /**
   * Its cross-reference label.
   */
  label: string;
  /**
   * The body with cross-references filled in (`§2.3 Services`).
   */
  markdown: string;
  /**
   * Its number in the book's style, if numbered.
   */
  number?: string | null;
  /**
   * Its file inside the book folder.
   */
  path: string;
  /**
   * Whether a restriction applies to it.
   */
  restricted: boolean;
  /**
   * The whole file, front matter included.
   */
  source: string;
  /**
   * Its title.
   */
  title: string;
}

/**
 * Names a section.
 */
export interface SectionKey {
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * The section (its front-matter ID).
   */
  section_id: string;
}

/**
 * One section of My Work.
 */
export interface SectionView {
  /**
   * The bucket.
   */
  bucket: BucketName;
  /**
   * For Upcoming: the day (`YYYY-MM-DD`) within the next seven.
   */
  day?: string | null;
  /**
   * The tasks, in display order.
   */
  tasks: TaskView[];
  /**
   * For Upcoming: the Monday of a later week.
   */
  week_of?: string | null;
}

/**
 * Admin-session security.
 */
export interface SecuritySettings {
  /**
   * Admin actions need a sign-in this recent, in minutes.
   */
  fresh_auth_minutes?: number;
  /**
   * Session lifetime in hours.
   */
  session_hours?: number;
}

/**
 * One select or multi-select option.
 */
export interface SelectOption {
  /**
   * A palette colour name for chips, if set.
   */
  color?: string | null;
  /**
   * The option text, as stored in row files.
   */
  name: string;
}

/**
 * One chart series.
 */
export interface Series {
  /**
   * The legend label.
   */
  name: string;
  /**
   * One value per category.
   */
  values: number[];
}

/**
 * How a browser session was established.
 */
export type SessionMethod = "password" | "break_glass" | "sso";

/**
 * A session as listed.
 */
export interface SessionView {
  /**
   * When it was created.
   */
  createdAt: Timestamp;
  /**
   * When it ends.
   */
  expiresAt: Timestamp;
  /**
   * The session ID.
   */
  id: string;
  /**
   * When it was last used, to within a few minutes.
   */
  lastSeenAt: Timestamp;
  /**
   * How it was established.
   */
  method: SessionMethod;
  /**
   * The browser that holds it.
   */
  userAgent?: string | null;
}

/**
 * `books.set_section_access`: replaces the section's own settings.
 */
export interface SetSectionAccess {
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * Its owners; empty inherits them.
   */
  owners?: string[];
  /**
   * Who may see it and everything under it (`user:<id>`,
   * `group:<id>`); empty lifts its own restriction. It can only narrow
   * an enclosing restriction.
   */
  restrict_to: string[];
  /**
   * Its reviewers; empty inherits them.
   */
  reviewers?: string[];
  /**
   * The section.
   */
  section_id: string;
}

/**
 * New settings and the version they replace.
 */
export interface SettingsUpdate {
  /**
   * The version read; the save fails if someone saved since.
   */
  expected_version: number;
  /**
   * The settings.
   */
  settings: AdminUpdateSettingsAdminSettings;
}

/**
 * A space's settings.
 */
export interface SettingsView {
  /**
   * Days one version a day is kept after that.
   */
  daily_days: number;
  /**
   * Days every version of a page is kept.
   */
  keep_all_days: number;
  /**
   * The most bytes the space may hold, trash and versions included;
   * no limit when absent.
   */
  quota_bytes?: number | null;
  /**
   * Days an item stays in the trash before it is purged.
   */
  trash_days: number;
  /**
   * Weeks one version a week is kept after that; forever when absent.
   */
  weekly_weeks?: number | null;
}

/**
 * The first administrator's details.
 */
export interface SetupInput {
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The display name.
   */
  name: string;
  /**
   * The password, at least 12 characters.
   */
  password: string;
}

/**
 * How serious a diagnostic is.
 */
export type Severity = "warning" | "error";

/**
 * An open issue with a similar title.
 */
export interface SimilarOut {
  /**
   * Its identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Shared title words, in percent.
   */
  overlap: number;
  /**
   * Its title.
   */
  title: string;
}

/**
 * An SLA clock.
 */
export interface SlaOut {
  /**
   * When it breaches.
   */
  breaches_at: string;
  /**
   * The SLA rule's name.
   */
  kind: string;
  /**
   * gray, yellow, orange or red.
   */
  risk: string;
  /**
   * When it stopped (closed or met).
   */
  stopped_at?: string | null;
}

/**
 * The SLOs.
 */
export interface SlosOutput {
  /**
   * Each SLO with its burn rates and remaining budget.
   */
  slos: unknown[];
}

/**
 * A snapshot's header.
 */
export interface SnapshotInfo {
  /**
   * When it was taken (RFC 3339).
   */
  created_at: string;
  /**
   * Who took it (`user:<id>`).
   */
  created_by: string;
  /**
   * What it marks.
   */
  message: string;
  /**
   * The tag, such as `v1.0`.
   */
  tag: string;
}

/**
 * A book's snapshots, by tag.
 */
export interface SnapshotList {
  /**
   * The snapshots.
   */
  snapshots: SnapshotInfo[];
}

/**
 * A SpaceId as a 26-character ULID.
 */
export type SpaceId = string;

/**
 * Names a space.
 */
export interface SpaceKey {
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * A space's key list: what new pages are sealed to.
 */
export interface SpaceKeys {
  /**
   * Every enrolled device, oldest first.
   */
  keys: DeviceKeyInfo[];
  /**
   * The space.
   */
  space_id: SpaceId;
  /**
   * Changes whenever the list changes; empty while the space has no
   * keys (it is not private).
   */
  version: string;
}

/**
 * What kind of space.
 */
export type SpaceKindView = "shared_drive" | "user_drive";

/**
 * Spaces.
 */
export interface SpaceList {
  /**
   * The spaces.
   */
  spaces: SpaceView[];
}

/**
 * A space, by ID.
 */
export interface SpaceRef {
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * A space.
 */
export interface SpaceView {
  /**
   * The space.
   */
  id: SpaceId;
  /**
   * Shared drive or My Drive.
   */
  kind: SpaceKindView;
  /**
   * The display name.
   */
  name: string;
  /**
   * The owner of a My Drive.
   */
  owner?: UserId | null;
  /**
   * Quota, trash and version settings.
   */
  settings: SettingsView;
  /**
   * The short name in its folder path.
   */
  slug: string;
}

/**
 * A range of characters (Unicode scalar values, not bytes) in the source,
 * so editors can underline the exact spot of an error.
 */
export interface Span {
  /**
   * One past the last character.
   */
  end: number;
  /**
   * First character, counting from 0.
   */
  start: number;
}

/**
 * How the sign-in page offers single sign-on.
 */
export interface SsoButton {
  /**
   * The provider's name, for "Sign in with {name}".
   */
  name: string;
  /**
   * Where the button goes; append `?next=<path>` to return there.
   */
  startUrl: string;
}

/**
 * Starring or unstarring an item.
 */
export interface StarInput {
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * Whether it is starred.
   */
  starred: boolean;
}

/**
 * Summary numbers for a set of issues.
 */
export interface StatsOut {
  /**
   * How many issues.
   */
  count: number;
  /**
   * Mean, in hours.
   */
  mean_hours?: number | null;
  /**
   * Median, in hours; absent for counts.
   */
  p50_hours?: number | null;
  /**
   * 85th percentile, in hours: most work finishes within this.
   */
  p85_hours?: number | null;
  /**
   * 95th percentile, in hours.
   */
  p95_hours?: number | null;
}

/**
 * A status in a team's workflow.
 */
export interface StatusOut {
  /**
   * One of triage, backlog, unstarted, started, completed, canceled,
   * duplicate.
   */
  category: string;
  /**
   * Slug, such as `in-review`.
   */
  key: string;
  /**
   * Display name.
   */
  name: string;
}

/**
 * A stored object.
 */
export interface Stored {
  /**
   * Its new version.
   */
  etag: string;
  /**
   * Its path inside the space folder.
   */
  path: string;
}

/**
 * A block as stored in an index: what a backend returns.
 */
export interface StoredBlock {
  /**
   * The block.
   */
  block: BlockId;
  /**
   * The block's kind.
   */
  block_kind: BlockKind;
  /**
   * The document.
   */
  doc: DocId;
  /**
   * The document's kind.
   */
  doc_kind: DocKind;
  /**
   * Headings above the block.
   */
  heading_path: string[];
  /**
   * The document's path.
   */
  path: string;
  /**
   * Position of the block in the document.
   */
  position: number;
  /**
   * The section heading.
   */
  section?: BlockId | null;
  /**
   * The space.
   */
  space: SpaceId;
  /**
   * The block's text.
   */
  text: string;
  /**
   * The document's title.
   */
  title: string;
}

/**
 * Answers to a form.
 */
export interface SubmitForm {
  /**
   * Answers by question key.
   */
  answers?: Record<string, string>;
  /**
   * The form's slug.
   */
  slug: string;
  /**
   * The team's key.
   */
  team: string;
}

/**
 * `PushSubscription.toJSON()`.
 */
export interface Subscribe {
  /**
   * The push service URL (`https` only).
   */
  endpoint: string;
  /**
   * When the browser will drop the subscription; not used.
   */
  expirationTime?: number | null;
  /**
   * The keys messages are encrypted to.
   */
  keys: PushKeys;
}

/**
 * A proposed triage decision for an issue.
 */
export interface SuggestTriage {
  /**
   * Who should own it (a handle).
   */
  assignee?: string | null;
  /**
   * The key of an open issue it repeats.
   */
  duplicate_of?: string | null;
  /**
   * The issue.
   */
  key: string;
  /**
   * Labels to add.
   */
  labels?: string[];
  /**
   * Which assistant is suggesting, as it names itself.
   */
  model?: string;
  /**
   * The priority, 0 (none) to 4 (low).
   */
  priority?: number | null;
  /**
   * Why, in a sentence or two; shown next to the suggestion.
   */
  reason: string;
  /**
   * The team to move it to.
   */
  team?: string | null;
}

/**
 * A proposed triage decision.
 */
export interface SuggestionOut {
  /**
   * Who should own the issue.
   */
  assignee?: string | null;
  /**
   * When it was made (RFC 3339).
   */
  at: string;
  /**
   * The issue this one repeats.
   */
  duplicate_of?: string | null;
  /**
   * Labels to add.
   */
  labels: string[];
  /**
   * Which assistant made it.
   */
  model: string;
  /**
   * The priority, 0 (none) to 4 (low).
   */
  priority?: number | null;
  /**
   * Why, in a sentence or two.
   */
  reason: string;
  /**
   * The team to move the issue to, if not its current one.
   */
  team?: string | null;
}

/**
 * What a sync did.
 */
export interface SyncView {
  /**
   * Tasks added from new events.
   */
  adopted: number;
  /**
   * Events created for tasks.
   */
  created: number;
  /**
   * Events deleted.
   */
  deleted: number;
  /**
   * Tasks moved because their event moved.
   */
  moved: number;
  /**
   * Tasks whose event was deleted, so they lost their block.
   */
  unblocked: number;
  /**
   * Events updated from tasks.
   */
  updated: number;
}

/**
 * What a thread is on.
 */
export interface TargetOut {
  /**
   * The block ID, for a block.
   */
  block?: string | null;
  /**
   * The document ID, for a page or block.
   */
  doc?: string | null;
  /**
   * The issue key, for an issue.
   */
  issue?: string | null;
}

/**
 * Where a task comes from.
 */
export type TaskKind = "checklist" | "issue" | "comment" | "mention" | "review";

/**
 * A task by ID.
 */
export interface TaskRef {
  /**
   * The task ID.
   */
  task: string;
}

/**
 * One task in a view.
 */
export interface TaskView {
  /**
   * People mentioned in the item, without `@`.
   */
  assignees: string[];
  /**
   * The document it lives in, for checklist items and comments.
   */
  doc?: DocId | null;
  /**
   * Whether it is completed.
   */
  done: boolean;
  /**
   * The fields.
   */
  fields: FieldsView;
  /**
   * The task ID: `<doc>.<block>` for checklist items.
   */
  id: string;
  /**
   * The issue key, for issues.
   */
  issue_key?: string | null;
  /**
   * Where it comes from.
   */
  kind: TaskKind;
  /**
   * The list's title, for items in task lists.
   */
  list?: string | null;
  /**
   * For a mention: who mentioned the person, without `@`.
   */
  mentioned_by?: string | null;
  /**
   * Whether its deadline has passed.
   */
  overdue: boolean;
  /**
   * Subtasks.
   */
  subtasks: TaskView[];
  /**
   * The title, as Vellum Markdown inline text.
   */
  title: string;
}

/**
 * A task list.
 */
export interface TasksCreateListListView {
  /**
   * The list's document.
   */
  doc: DocId;
  /**
   * Whether it is the person's Inbox.
   */
  inbox: boolean;
  /**
   * Members' handles.
   */
  members: string[];
  /**
   * How many open items it holds.
   */
  open: number;
  /**
   * The owner's handle.
   */
  owner?: string | null;
  /**
   * The title.
   */
  title: string;
}

/**
 * One task in a view.
 */
export interface TasksCreateTaskView {
  /**
   * People mentioned in the item, without `@`.
   */
  assignees: string[];
  /**
   * The document it lives in, for checklist items and comments.
   */
  doc?: DocId | null;
  /**
   * Whether it is completed.
   */
  done: boolean;
  /**
   * The fields.
   */
  fields: FieldsView;
  /**
   * The task ID: `<doc>.<block>` for checklist items.
   */
  id: string;
  /**
   * The issue key, for issues.
   */
  issue_key?: string | null;
  /**
   * Where it comes from.
   */
  kind: TaskKind;
  /**
   * The list's title, for items in task lists.
   */
  list?: string | null;
  /**
   * For a mention: who mentioned the person, without `@`.
   */
  mentioned_by?: string | null;
  /**
   * Whether its deadline has passed.
   */
  overdue: boolean;
  /**
   * Subtasks.
   */
  subtasks: TasksCreateTaskView[];
  /**
   * The title, as Vellum Markdown inline text.
   */
  title: string;
}

/**
 * No input.
 */
export type TasksDigestNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksGoogleConnectNoInput = Record<string, unknown>;

/**
 * An acknowledgement with nothing to return.
 */
export type TasksGoogleDisconnectDone = Record<string, unknown>;

/**
 * No input.
 */
export type TasksGoogleDisconnectNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksGoogleStatusNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksGoogleSyncNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksListsNoInput = Record<string, unknown>;

/**
 * An acknowledgement with nothing to return.
 */
export type TasksMentionDismissDone = Record<string, unknown>;

/**
 * One task in a view.
 */
export interface TasksMoveTaskView {
  /**
   * People mentioned in the item, without `@`.
   */
  assignees: string[];
  /**
   * The document it lives in, for checklist items and comments.
   */
  doc?: DocId | null;
  /**
   * Whether it is completed.
   */
  done: boolean;
  /**
   * The fields.
   */
  fields: FieldsView;
  /**
   * The task ID: `<doc>.<block>` for checklist items.
   */
  id: string;
  /**
   * The issue key, for issues.
   */
  issue_key?: string | null;
  /**
   * Where it comes from.
   */
  kind: TaskKind;
  /**
   * The list's title, for items in task lists.
   */
  list?: string | null;
  /**
   * For a mention: who mentioned the person, without `@`.
   */
  mentioned_by?: string | null;
  /**
   * Whether its deadline has passed.
   */
  overdue: boolean;
  /**
   * Subtasks.
   */
  subtasks: TasksMoveTaskView[];
  /**
   * The title, as Vellum Markdown inline text.
   */
  title: string;
}

/**
 * No input.
 */
export type TasksMyWorkNoInput = Record<string, unknown>;

/**
 * What a recognised quick-add phrase set.
 */
export type TasksParseTokenView = "due" | "start" | "recurrence" | "when" | "list" | "label" | "person" | "priority" | "reminder";

/**
 * One task in a view.
 */
export interface TasksReopenTaskView {
  /**
   * People mentioned in the item, without `@`.
   */
  assignees: string[];
  /**
   * The document it lives in, for checklist items and comments.
   */
  doc?: DocId | null;
  /**
   * Whether it is completed.
   */
  done: boolean;
  /**
   * The fields.
   */
  fields: FieldsView;
  /**
   * The task ID: `<doc>.<block>` for checklist items.
   */
  id: string;
  /**
   * The issue key, for issues.
   */
  issue_key?: string | null;
  /**
   * Where it comes from.
   */
  kind: TaskKind;
  /**
   * The list's title, for items in task lists.
   */
  list?: string | null;
  /**
   * For a mention: who mentioned the person, without `@`.
   */
  mentioned_by?: string | null;
  /**
   * Whether its deadline has passed.
   */
  overdue: boolean;
  /**
   * Subtasks.
   */
  subtasks: TasksReopenTaskView[];
  /**
   * The title, as Vellum Markdown inline text.
   */
  title: string;
}

/**
 * An acknowledgement with nothing to return.
 */
export type TasksRevokeCaptureDone = Record<string, unknown>;

/**
 * No input.
 */
export type TasksRevokeCaptureNoInput = Record<string, unknown>;

/**
 * An acknowledgement with nothing to return.
 */
export type TasksRevokeFeedDone = Record<string, unknown>;

/**
 * No input.
 */
export type TasksRevokeFeedNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksRotateCaptureNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksRotateFeedNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type TasksSettingsNoInput = Record<string, unknown>;

/**
 * A task list.
 */
export interface TasksShareListListView {
  /**
   * The list's document.
   */
  doc: DocId;
  /**
   * Whether it is the person's Inbox.
   */
  inbox: boolean;
  /**
   * Members' handles.
   */
  members: string[];
  /**
   * How many open items it holds.
   */
  open: number;
  /**
   * The owner's handle.
   */
  owner?: string | null;
  /**
   * The title.
   */
  title: string;
}

/**
 * A task list.
 */
export interface TasksUnshareListListView {
  /**
   * The list's document.
   */
  doc: DocId;
  /**
   * Whether it is the person's Inbox.
   */
  inbox: boolean;
  /**
   * Members' handles.
   */
  members: string[];
  /**
   * How many open items it holds.
   */
  open: number;
  /**
   * The owner's handle.
   */
  owner?: string | null;
  /**
   * The title.
   */
  title: string;
}

/**
 * One task in a view.
 */
export interface TasksUpdateTaskView {
  /**
   * People mentioned in the item, without `@`.
   */
  assignees: string[];
  /**
   * The document it lives in, for checklist items and comments.
   */
  doc?: DocId | null;
  /**
   * Whether it is completed.
   */
  done: boolean;
  /**
   * The fields.
   */
  fields: FieldsView;
  /**
   * The task ID: `<doc>.<block>` for checklist items.
   */
  id: string;
  /**
   * The issue key, for issues.
   */
  issue_key?: string | null;
  /**
   * Where it comes from.
   */
  kind: TaskKind;
  /**
   * The list's title, for items in task lists.
   */
  list?: string | null;
  /**
   * For a mention: who mentioned the person, without `@`.
   */
  mentioned_by?: string | null;
  /**
   * Whether its deadline has passed.
   */
  overdue: boolean;
  /**
   * Subtasks.
   */
  subtasks: TasksUpdateTaskView[];
  /**
   * The title, as Vellum Markdown inline text.
   */
  title: string;
}

/**
 * A team an issue could go to, in a triage brief.
 */
export interface TeamOptionOut {
  /**
   * The team key.
   */
  key: string;
  /**
   * Labels usable on the team's issues.
   */
  labels: string[];
  /**
   * The team name.
   */
  name: string;
  /**
   * People who can be assigned.
   */
  people: string[];
}

/**
 * A team and its workflow.
 */
export interface TeamOut {
  /**
   * Whether the team runs cycles.
   */
  cycles: boolean;
  /**
   * The status new issues get.
   */
  default_status: string;
  /**
   * Team ID.
   */
  id: string;
  /**
   * Key prefix, such as `ENG`.
   */
  key: string;
  /**
   * Label references (`group/name` or `name`).
   */
  labels: string[];
  /**
   * The team lead's handle.
   */
  lead?: string | null;
  /**
   * Member handles.
   */
  members: string[];
  /**
   * Display name.
   */
  name: string;
  /**
   * Statuses in board order.
   */
  statuses: StatusOut[];
  /**
   * Template names.
   */
  templates: string[];
  /**
   * IANA time zone for cycles and recurring work.
   */
  timezone: string;
  /**
   * Whether intake issues go to triage first.
   */
  triage: boolean;
}

/**
 * A team by key.
 */
export interface TeamRef {
  /**
   * Team key, such as `ENG`.
   */
  team: string;
}

/**
 * Every team.
 */
export interface TeamsOut {
  /**
   * Teams by key.
   */
  teams: TeamOut[];
}

/**
 * Opt-in anonymous usage reporting.
 */
export interface TelemetrySettings {
  /**
   * Whether reports are sent. Off until an admin turns it on.
   */
  enabled?: boolean;
  /**
   * Where reports go (HTTPS).
   */
  endpoint?: string | null;
  /**
   * A random id for this installation, unrelated to anything else.
   */
  install_id?: string;
  /**
   * Hours between reports.
   */
  interval_hours?: number;
}

/**
 * A comment thread.
 */
export interface ThreadOut {
  /**
   * Assignee handle ("assign to").
   */
  assignee?: string | null;
  /**
   * Comments, oldest first.
   */
  comments: CommentOut[];
  /**
   * Thread ID (the first comment's ID).
   */
  id: string;
  /**
   * The quoted text.
   */
  quote?: string | null;
  /**
   * When resolved; unset while open.
   */
  resolved_at?: string | null;
  /**
   * Who resolved it.
   */
  resolved_by?: string | null;
  /**
   * What the thread is on.
   */
  target: TargetOut;
}

/**
 * Threads on one document or issue.
 */
export interface ThreadsOut {
  /**
   * Open threads first, then resolved, each oldest first.
   */
  threads: ThreadOut[];
}

export type Timestamp = string;

/**
 * A title match for the instant (⌘K) search.
 */
export interface TitleHit {
  /**
   * The document.
   */
  doc: DocId;
  /**
   * Its kind.
   */
  kind: DocKind;
  /**
   * Its path.
   */
  path: string;
  /**
   * The space.
   */
  space: SpaceId;
  /**
   * Its title.
   */
  title: string;
}

/**
 * `search.titles` input.
 */
export interface TitlesInput {
  /**
   * Matches to return (default 10, at most 100).
   */
  limit?: number | null;
  /**
   * What the person has typed so far.
   */
  q: string;
  /**
   * Comma-separated space IDs to search in.
   */
  spaces?: string | null;
}

/**
 * `search.titles` output.
 */
export interface TitlesPage {
  /**
   * The matching titles, best first.
   */
  hits: TitleHit[];
}

/**
 * A line of the table of contents.
 */
export type TocLine = TocNode | {
  kind: "part";
  /**
   * The title.
   */
  title: string;
} | {
  kind: "separator";
};

/**
 * One chapter of the table of contents.
 */
export interface TocNode {
  /**
   * Nested chapters.
   */
  children: TocNode[];
  /**
   * Nesting depth, 1 at the top.
   */
  depth: number;
  /**
   * Whether the chapter is a draft without a file.
   */
  draft: boolean;
  /**
   * The section ID; absent for a draft.
   */
  id?: string | null;
  /**
   * The cross-reference label, such as `§2.3 Services`.
   */
  label: string;
  /**
   * The number in the book's style (`2.3`, `II.C`), if numbered.
   */
  number?: string | null;
  /**
   * The part this chapter falls under.
   */
  part?: string | null;
  /**
   * The chapter file inside the book folder; absent for a draft.
   */
  path?: string | null;
  /**
   * The navigation title.
   */
  title: string;
}

/**
 * An API token as listed (never the secret).
 */
export interface TokenView {
  /**
   * When it was created.
   */
  createdAt: Timestamp;
  /**
   * When it stops working, if ever.
   */
  expiresAt?: Timestamp | null;
  /**
   * The token ID.
   */
  id: string;
  /**
   * When it was last used, to within a few minutes.
   */
  lastUsedAt?: Timestamp | null;
  /**
   * Its label.
   */
  name: string;
  /**
   * The granted scopes.
   */
  scopes: Scope[];
}

/**
 * One trace.
 */
export interface TraceInput {
  /**
   * The 32-hex-digit W3C trace id.
   */
  trace_id: string;
}

/**
 * A space's tree, or the part of it under one item.
 */
export interface TreeInput {
  /**
   * How many levels to return; all when absent.
   */
  depth?: number | null;
  /**
   * At most this many items, 1 to 10000; 2000 when absent.
   */
  limit?: number | null;
  /**
   * The folder or page to start under; the whole space when absent.
   */
  root_id?: ObjectId | null;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * Items in tree order: depth first, siblings in their stored order.
 * Each item names its parent.
 */
export interface TreeView {
  /**
   * The items you can open.
   */
  items: NodeView[];
  /**
   * True when the limit cut the tree short.
   */
  truncated: boolean;
}

/**
 * A triage decision.
 */
export interface TriageIssue {
  /**
   * `accept`, `decline`, `duplicate` or `snooze`.
   */
  action: string;
  /**
   * For duplicate: the original issue's key.
   */
  duplicate_of?: string | null;
  /**
   * The issue.
   */
  key: string;
  /**
   * For decline: why.
   */
  reason?: string | null;
  /**
   * For accept: the status to move to.
   */
  state?: string | null;
  /**
   * For snooze: until when (RFC 3339).
   */
  until?: string | null;
}

/**
 * The unread count after a change.
 */
export interface Unread {
  /**
   * How many lines are unread.
   */
  unread: number;
}

/**
 * Which browser to forget.
 */
export interface Unsubscribe {
  /**
   * The push service URL the browser registered.
   */
  endpoint: string;
}

/**
 * Changes to a task. Each field present replaces the old value; an
 * empty string clears it.
 */
export interface UpdateInput {
  /**
   * A time block, `YYYY-MM-DDTHH:MM/<minutes>`, shown in Google
   * Calendar when it is connected.
   */
  block?: string | null;
  /**
   * `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.
   */
  due?: string | null;
  /**
   * Comma-separated labels.
   */
  labels?: string | null;
  /**
   * `1` to `4`.
   */
  priority?: string | null;
  /**
   * `schedule` or `completion`.
   */
  recur_from?: string | null;
  /**
   * Comma-separated reminders, such as `-PT15M,2026-11-03T09:00`.
   */
  remind?: string | null;
  /**
   * An RFC 5545 rule such as `FREQ=WEEKLY;BYDAY=MO`.
   */
  rrule?: string | null;
  /**
   * `YYYY-MM-DD`.
   */
  start?: string | null;
  /**
   * The task ID.
   */
  task: string;
  /**
   * The new title.
   */
  title?: string | null;
  /**
   * `evening`, `anytime` or `someday`.
   */
  when?: string | null;
}

/**
 * Changes to an issue. Absent fields are kept; an empty string clears.
 */
export interface UpdateIssue {
  /**
   * Labels to add.
   */
  add_labels?: string[];
  /**
   * Relations to add.
   */
  add_relations?: RelationIn[];
  /**
   * New assignee handle; empty clears.
   */
  assignee?: string | null;
  /**
   * Cycle key; empty clears.
   */
  cycle?: string | null;
  /**
   * New description.
   */
  description?: string | null;
  /**
   * Due date; empty clears.
   */
  due?: string | null;
  /**
   * New estimate; empty clears.
   */
  estimate?: string | null;
  /**
   * The issue (`ENG-142`, alias or ID).
   */
  key: string;
  /**
   * Parent issue key; empty clears.
   */
  parent?: string | null;
  /**
   * New priority 0–4.
   */
  priority?: number | null;
  /**
   * Project ID; empty clears.
   */
  project?: string | null;
  /**
   * Labels to remove.
   */
  remove_labels?: string[];
  /**
   * Relations to remove.
   */
  remove_relations?: RelationIn[];
  /**
   * New status slug.
   */
  state?: string | null;
  /**
   * Subscribe (`true`) or unsubscribe the caller.
   */
  subscribe?: boolean | null;
  /**
   * New title.
   */
  title?: string | null;
}

/**
 * Profile changes.
 */
export interface UpdateMe {
  /**
   * The new display name.
   */
  name: string;
}

/**
 * Changes some values of a row, and optionally its body.
 */
export interface UpdateRowInput {
  /**
   * A new body, when it changes.
   */
  body?: string | null;
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * The row.
   */
  row_id: DocId;
  /**
   * Values by property key or name; an `empty` value clears one.
   */
  values?: Record<string, Value>;
}

/**
 * `books.update_section`.
 */
export interface UpdateSection {
  /**
   * A new Markdown body; the front matter is kept.
   */
  body?: string | null;
  /**
   * The book.
   */
  book_id: BookId;
  /**
   * The section.
   */
  section_id: string;
  /**
   * A new title (renames the file too).
   */
  title?: string | null;
}

/**
 * Changes to a space.
 */
export interface UpdateSpace {
  /**
   * A new display name; unchanged when absent.
   */
  name?: string | null;
  /**
   * The new settings, replacing the old ones.
   */
  settings: SettingsView;
  /**
   * The space.
   */
  space_id: SpaceId;
}

/**
 * Changes an administrator makes to a user.
 */
export interface UpdateUser {
  /**
   * Grant or remove administrator rights.
   */
  admin?: boolean | null;
  /**
   * Block or allow sign-in. Blocking ends every session and token.
   */
  disabled?: boolean | null;
  /**
   * The user's ID.
   */
  id: UserId;
  /**
   * A new display name.
   */
  name?: string | null;
}

/**
 * Exactly what is sent.
 */
export interface UsageReport {
  /**
   * `x86_64` or `aarch64`.
   */
  arch: string;
  /**
   * The database.
   */
  database: string;
  /**
   * Size bucket of documents.
   */
  documents: string;
  /**
   * Optional features turned on, sorted.
   */
  features: string[];
  /**
   * The random installation id.
   */
  install_id: string;
  /**
   * `linux`, `macos` or `windows`.
   */
  os: string;
  /**
   * The deployment profile.
   */
  profile: string;
  /**
   * [`REPORT_SCHEMA`].
   */
  schema: number;
  /**
   * Size bucket of spaces.
   */
  spaces: string;
  /**
   * Size bucket of active people.
   */
  users: string;
  /**
   * The Vellum version.
   */
  version: string;
}

/**
 * How much of its quota a space uses.
 */
export interface UsageView {
  /**
   * The quota; no limit when absent.
   */
  quota_bytes?: number | null;
  /**
   * Bytes used, trash and versions included.
   */
  used_bytes: number;
}

/**
 * A UserId as a 26-character ULID.
 */
export type UserId = string;

/**
 * A user as the API shows it.
 */
export interface UserView {
  /**
   * Whether the user administers the workspace.
   */
  admin: boolean;
  /**
   * When the account was created.
   */
  createdAt: Timestamp;
  /**
   * Whether sign-in is blocked.
   */
  disabled: boolean;
  /**
   * The sign-in address.
   */
  email: string;
  /**
   * The unique handle used for assignees and mentions.
   */
  handle: string;
  /**
   * Whether the account has a password (otherwise SSO only).
   */
  hasPassword: boolean;
  /**
   * The user's ID.
   */
  id: UserId;
  /**
   * The display name.
   */
  name: string;
}

/**
 * A property value.
 *
 * Serialised for the API as `{"type": "number", "value": 3}`.
 */
export type Value = {
  type: "empty";
} | {
  type: "text";
  value: string;
} | {
  type: "number";
  value: number;
} | {
  type: "bool";
  value: boolean;
} | {
  type: "date";
  value: DateValue;
} | {
  type: "list";
  value: Value[];
};

/**
 * One version's content.
 */
export interface VersionContent {
  /**
   * The text; absent if it is not UTF-8 text.
   */
  content?: string | null;
  /**
   * The version.
   */
  id: VersionId;
}

/**
 * A VersionId as a 26-character ULID.
 */
export type VersionId = string;

/**
 * Names one version of a page.
 */
export interface VersionKey {
  /**
   * The page.
   */
  id: ObjectId;
  /**
   * The version.
   */
  version_id: VersionId;
}

/**
 * A page's versions, oldest first.
 */
export interface VersionList {
  /**
   * The versions.
   */
  versions: VersionView[];
}

/**
 * One saved version of a page.
 */
export interface VersionView {
  /**
   * Who wrote it.
   */
  author: ActorView;
  /**
   * The version.
   */
  id: VersionId;
  /**
   * Its size in bytes.
   */
  size: number;
}

/**
 * Settings with the version they were stored at.
 */
export interface Versioned {
  /**
   * The settings.
   */
  settings: AdminSettings;
  /**
   * Increases by one on every save; 0 before the first.
   */
  version: number;
}

/**
 * A saved view, in the text form `schema.yaml` uses.
 */
export interface ViewInfo {
  /**
   * Date property for calendar and timeline.
   */
  date?: string | null;
  /**
   * End-date property for timeline bars.
   */
  end?: string | null;
  /**
   * Filters, all of which must match.
   */
  filters: string[];
  /**
   * Group-by property key.
   */
  group?: string | null;
  /**
   * Hidden property keys.
   */
  hidden: string[];
  /**
   * The layout.
   */
  kind: ViewKind;
  /**
   * Most rows shown.
   */
  limit?: number | null;
  /**
   * The view's name.
   */
  name: string;
  /**
   * Sort keys, most significant first.
   */
  sorts: string[];
}

/**
 * Names one saved view.
 */
export interface ViewKey {
  /**
   * The collection.
   */
  collection_id: CollectionId;
  /**
   * The view's name (matched without regard to case).
   */
  view: string;
}

/**
 * How a view lays out rows.
 */
export type ViewKind = "table" | "board" | "gallery" | "calendar" | "timeline" | "list";

/**
 * The result of running a view.
 */
export interface ViewResult {
  /**
   * The collection.
   */
  collection: CollectionId;
  /**
   * Visible columns, title first.
   */
  columns: Column[];
  /**
   * Groups; a single unlabelled group when the view is not grouped.
   */
  groups: Group[];
  /**
   * The layout.
   */
  kind: ViewKind;
  /**
   * The collection title.
   */
  title: string;
  /**
   * Rows matching the filters, before the limit.
   */
  total: number;
  /**
   * Whether the limit cut rows off.
   */
  truncated: boolean;
  /**
   * The view name.
   */
  view: string;
}

/**
 * A window in minutes.
 */
export interface WindowInput {
  /**
   * Minutes, 1 to 60 (default 15).
   */
  minutes?: number | null;
}

/**
 * An issue.
 */
export interface WorkAcceptSuggestionIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * A team and its workflow.
 */
export interface WorkCreateTeamTeamOut {
  /**
   * Whether the team runs cycles.
   */
  cycles: boolean;
  /**
   * The status new issues get.
   */
  default_status: string;
  /**
   * Team ID.
   */
  id: string;
  /**
   * Key prefix, such as `ENG`.
   */
  key: string;
  /**
   * Label references (`group/name` or `name`).
   */
  labels: string[];
  /**
   * The team lead's handle.
   */
  lead?: string | null;
  /**
   * Member handles.
   */
  members: string[];
  /**
   * Display name.
   */
  name: string;
  /**
   * Statuses in board order.
   */
  statuses: StatusOut[];
  /**
   * Template names.
   */
  templates: string[];
  /**
   * IANA time zone for cycles and recurring work.
   */
  timezone: string;
  /**
   * Whether intake issues go to triage first.
   */
  triage: boolean;
}

/**
 * An issue.
 */
export interface WorkDismissSuggestionIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * An issue.
 */
export interface WorkGetIssueIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * A project with its computed progress.
 */
export interface WorkGetProjectProjectOut {
  /**
   * Project ID.
   */
  id: string;
  /**
   * Issues in the project.
   */
  issues: number;
  /**
   * Lead handle.
   */
  lead?: string | null;
  /**
   * The project file, front matter and spec.
   */
  markdown: string;
  /**
   * Percent complete by points.
   */
  percent: number;
  /**
   * planned, started, paused, completed or canceled.
   */
  status: string;
  /**
   * Target date.
   */
  target_date?: string | null;
  /**
   * Team keys.
   */
  teams: string[];
  /**
   * Title.
   */
  title: string;
}

/**
 * A team and its workflow.
 */
export interface WorkGetTeamTeamOut {
  /**
   * Whether the team runs cycles.
   */
  cycles: boolean;
  /**
   * The status new issues get.
   */
  default_status: string;
  /**
   * Team ID.
   */
  id: string;
  /**
   * Key prefix, such as `ENG`.
   */
  key: string;
  /**
   * Label references (`group/name` or `name`).
   */
  labels: string[];
  /**
   * The team lead's handle.
   */
  lead?: string | null;
  /**
   * Member handles.
   */
  members: string[];
  /**
   * Display name.
   */
  name: string;
  /**
   * Statuses in board order.
   */
  statuses: StatusOut[];
  /**
   * Template names.
   */
  templates: string[];
  /**
   * IANA time zone for cycles and recurring work.
   */
  timezone: string;
  /**
   * Whether intake issues go to triage first.
   */
  triage: boolean;
}

/**
 * No input.
 */
export type WorkListProjectsNoInput = Record<string, unknown>;

/**
 * No input.
 */
export type WorkListTeamsNoInput = Record<string, unknown>;

/**
 * An issue.
 */
export interface WorkMoveIssueIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * No input.
 */
export type WorkMyAssignmentsNoInput = Record<string, unknown>;

/**
 * A project with its computed progress.
 */
export interface WorkPutProjectProjectOut {
  /**
   * Project ID.
   */
  id: string;
  /**
   * Issues in the project.
   */
  issues: number;
  /**
   * Lead handle.
   */
  lead?: string | null;
  /**
   * The project file, front matter and spec.
   */
  markdown: string;
  /**
   * Percent complete by points.
   */
  percent: number;
  /**
   * planned, started, paused, completed or canceled.
   */
  status: string;
  /**
   * Target date.
   */
  target_date?: string | null;
  /**
   * Team keys.
   */
  teams: string[];
  /**
   * Title.
   */
  title: string;
}

/**
 * A team and its workflow.
 */
export interface WorkPutTeamTeamOut {
  /**
   * Whether the team runs cycles.
   */
  cycles: boolean;
  /**
   * The status new issues get.
   */
  default_status: string;
  /**
   * Team ID.
   */
  id: string;
  /**
   * Key prefix, such as `ENG`.
   */
  key: string;
  /**
   * Label references (`group/name` or `name`).
   */
  labels: string[];
  /**
   * The team lead's handle.
   */
  lead?: string | null;
  /**
   * Member handles.
   */
  members: string[];
  /**
   * Display name.
   */
  name: string;
  /**
   * Statuses in board order.
   */
  statuses: StatusOut[];
  /**
   * Template names.
   */
  templates: string[];
  /**
   * IANA time zone for cycles and recurring work.
   */
  timezone: string;
  /**
   * Whether intake issues go to triage first.
   */
  triage: boolean;
}

/**
 * An issue.
 */
export interface WorkSubmitFormIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * A proposed triage decision.
 */
export interface WorkSuggestTriageSuggestionOut {
  /**
   * Who should own the issue.
   */
  assignee?: string | null;
  /**
   * When it was made (RFC 3339).
   */
  at: string;
  /**
   * The issue this one repeats.
   */
  duplicate_of?: string | null;
  /**
   * Labels to add.
   */
  labels: string[];
  /**
   * Which assistant made it.
   */
  model: string;
  /**
   * The priority, 0 (none) to 4 (low).
   */
  priority?: number | null;
  /**
   * Why, in a sentence or two.
   */
  reason: string;
  /**
   * The team to move the issue to, if not its current one.
   */
  team?: string | null;
}

/**
 * An issue.
 */
export interface WorkTriageIssueIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/**
 * An issue.
 */
export interface WorkUpdateIssueIssueOut {
  /**
   * Earlier identifiers that still resolve.
   */
  aliases: string[];
  /**
   * Assignee handle.
   */
  assignee?: string | null;
  /**
   * Canceled.
   */
  canceled_at?: string | null;
  /**
   * The status's category.
   */
  category: string;
  /**
   * Completed.
   */
  completed_at?: string | null;
  /**
   * Created.
   */
  created_at: string;
  /**
   * Creator handle.
   */
  creator?: string | null;
  /**
   * Cycle key, such as `2026-C44`.
   */
  cycle?: string | null;
  /**
   * The description, in Vellum Markdown; left out of lists.
   */
  description?: string | null;
  /**
   * Due date.
   */
  due?: string | null;
  /**
   * Estimate as written (`3`, `M`).
   */
  estimate?: string | null;
  /**
   * Issue ID; stable across moves.
   */
  id: string;
  /**
   * Identifier, such as `ENG-142`.
   */
  key: string;
  /**
   * Label references.
   */
  labels: string[];
  /**
   * Links: pull requests, the source block, documents.
   */
  links: LinkOut[];
  /**
   * Milestone ID.
   */
  milestone?: string | null;
  /**
   * Parent issue key.
   */
  parent?: string | null;
  /**
   * Priority 0 (none) to 4 (low); 1 is urgent.
   */
  priority: number;
  /**
   * Project ID.
   */
  project?: string | null;
  /**
   * Relations to other issues.
   */
  relations: RelationOut[];
  /**
   * The running or last SLA.
   */
  sla?: SlaOut | null;
  /**
   * Work started.
   */
  started_at?: string | null;
  /**
   * Status slug.
   */
  state: string;
  /**
   * Subscriber handles.
   */
  subscribers: string[];
  /**
   * Team key.
   */
  team: string;
  /**
   * Title.
   */
  title: string;
  /**
   * Last updated.
   */
  updated_at: string;
}

/** Every operation: its method, path, input and output. */
export interface Operations {
  /** The organisation's seed colour and export and site themes */
  "admin.appearance": {
    method: "GET";
    path: "/v1/appearance";
    input: AdminAppearanceEmpty;
    output: AppearanceSettings;
  };
  /** Browser timings for a page view or slow call, joined to its trace */
  "admin.client_report": {
    method: "POST";
    path: "/v1/telemetry/client";
    input: AnyValue;
    output: IngestedOutput;
  };
  /** Takes an encrypted backup now */
  "admin.create_backup": {
    method: "POST";
    path: "/v1/admin/backups";
    input: AdminCreateBackupEmpty;
    output: AdminCreateBackupBackupInfo;
  };
  /** The whole audit log as JSON Lines; the export is audited */
  "admin.export_audit": {
    method: "GET";
    path: "/v1/admin/audit/export";
    input: AdminExportAuditEmpty;
    output: AuditExport;
  };
  /** One request's trace with every span */
  "admin.get_request": {
    method: "GET";
    path: "/v1/admin/requests/{trace_id}";
    input: TraceInput;
    output: AnyValue;
  };
  /** The admin settings and their version */
  "admin.get_settings": {
    method: "GET";
    path: "/v1/admin/settings";
    input: AdminGetSettingsEmpty;
    output: Versioned;
  };
  /** Audit records, newest first */
  "admin.list_audit": {
    method: "GET";
    path: "/v1/admin/audit";
    input: AuditQuery;
    output: AuditList;
  };
  /** Backups on disk, newest first */
  "admin.list_backups": {
    method: "GET";
    path: "/v1/admin/backups";
    input: AdminListBackupsEmpty;
    output: BackupList;
  };
  /** Recent requests from the trace buffer, newest first */
  "admin.list_requests": {
    method: "GET";
    path: "/v1/admin/requests";
    input: RequestsInput;
    output: RequestsOutput;
  };
  /** Folded span stacks of recent requests, for vellum profile */
  "admin.profile": {
    method: "GET";
    path: "/v1/admin/profile";
    input: ProfileInput;
    output: ProfileResponse;
  };
  /** Rate, errors, latency and saturation per route, against its budget */
  "admin.routes": {
    method: "GET";
    path: "/v1/admin/routes";
    input: WindowInput;
    output: RoutesView;
  };
  /** Restores the newest backup into scratch space and checks it */
  "admin.run_drill": {
    method: "POST";
    path: "/v1/admin/backups/drill";
    input: AdminRunDrillEmpty;
    output: AdminRunDrillDrillReport;
  };
  /** Service level objectives, burn rates and remaining error budget */
  "admin.slos": {
    method: "GET";
    path: "/v1/admin/slos";
    input: AdminSlosEmpty;
    output: SlosOutput;
  };
  /** When backups, drills and telemetry last ran */
  "admin.state": {
    method: "GET";
    path: "/v1/admin/state";
    input: AdminStateEmpty;
    output: AdminState;
  };
  /** Exactly what anonymous telemetry would send */
  "admin.telemetry_preview": {
    method: "GET";
    path: "/v1/admin/telemetry/preview";
    input: AdminTelemetryPreviewEmpty;
    output: UsageReport;
  };
  /** Replaces the settings if nobody saved since expected_version */
  "admin.update_settings": {
    method: "PUT";
    path: "/v1/admin/settings";
    input: SettingsUpdate;
    output: AdminUpdateSettingsVersioned;
  };
  /** Checks the audit log's hash chain end to end */
  "admin.verify_audit": {
    method: "POST";
    path: "/v1/admin/audit/verify";
    input: AdminVerifyAuditEmpty;
    output: ChainReport;
  };
  /** Add a section */
  "books.add_section": {
    method: "POST";
    path: "/v1/books/{book_id}/sections";
    input: AddSection;
    output: SectionDoc;
  };
  /** Tag the current book */
  "books.create_snapshot": {
    method: "POST";
    path: "/v1/books/{book_id}/snapshots";
    input: CreateSnapshot;
    output: BooksCreateSnapshotSnapshotInfo;
  };
  /** Compare a snapshot with another snapshot or the current book */
  "books.diff": {
    method: "GET";
    path: "/v1/books/{book_id}/diff";
    input: DiffInput;
    output: DiffOutput;
  };
  /** Get a book's settings and table of contents */
  "books.get": {
    method: "GET";
    path: "/v1/books/{book_id}";
    input: BookKey;
    output: BookInfo;
  };
  /** Get one section with cross-references filled in */
  "books.get_section": {
    method: "GET";
    path: "/v1/books/{book_id}/sections/{section_id}";
    input: SectionKey;
    output: SectionDoc;
  };
  /** List the books you can open */
  "books.list": {
    method: "GET";
    path: "/v1/books";
    input: ListBooks;
    output: BookList;
  };
  /** List a book's tagged snapshots */
  "books.list_snapshots": {
    method: "GET";
    path: "/v1/books/{book_id}/snapshots";
    input: BookKey;
    output: SnapshotList;
  };
  /** Move or nest a section with everything under it */
  "books.move_section": {
    method: "POST";
    path: "/v1/books/{book_id}/sections/{section_id}/move";
    input: MoveSection;
    output: SectionDoc;
  };
  /** Remove a section with everything under it */
  "books.remove_section": {
    method: "DELETE";
    path: "/v1/books/{book_id}/sections/{section_id}";
    input: SectionKey;
    output: BooksRemoveSectionRemoved;
  };
  /** Restrict a section and set its owners and reviewers */
  "books.set_section_access": {
    method: "PUT";
    path: "/v1/books/{book_id}/sections/{section_id}/access";
    input: SetSectionAccess;
    output: BooksSetSectionAccessSectionAccessInfo;
  };
  /** Rename a section or replace its body */
  "books.update_section": {
    method: "PATCH";
    path: "/v1/books/{book_id}/sections/{section_id}";
    input: UpdateSection;
    output: SectionDoc;
  };
  /** Compute a chart's data from a collection */
  "collections.chart": {
    method: "GET";
    path: "/v1/collections/chart";
    input: ChartInput;
    output: ChartData;
  };
  /** Create a collection from a CSV file, inferring column types */
  "collections.create_from_csv": {
    method: "POST";
    path: "/v1/collections";
    input: CreateFromCsvInput;
    output: CollectionsCreateFromCsvCsvImported;
  };
  /** Add a row */
  "collections.create_row": {
    method: "POST";
    path: "/v1/collections/{collection_id}/rows";
    input: CreateRowInput;
    output: RowWritten;
  };
  /** Delete a row and unlink it */
  "collections.delete_row": {
    method: "DELETE";
    path: "/v1/collections/{collection_id}/rows/{row_id}";
    input: RowKey;
    output: CollectionsDeleteRowRowWritten;
  };
  /** Delete a saved view */
  "collections.delete_view": {
    method: "DELETE";
    path: "/v1/collections/{collection_id}/views/{view}";
    input: ViewKey;
    output: CollectionInfo;
  };
  /** Show the live rows of a collection block in a page */
  "collections.embed": {
    method: "GET";
    path: "/v1/collections/embed";
    input: EmbedInput;
    output: EmbedResult;
  };
  /** Export a view as CSV */
  "collections.export_csv": {
    method: "GET";
    path: "/v1/collections/{collection_id}/csv";
    input: ExportCsvInput;
    output: CsvExport;
  };
  /** Get a collection's properties and saved views */
  "collections.get": {
    method: "GET";
    path: "/v1/collections/{collection_id}";
    input: CollectionKey;
    output: CollectionInfo;
  };
  /** Get one row with its computed values */
  "collections.get_row": {
    method: "GET";
    path: "/v1/collections/{collection_id}/rows/{row_id}";
    input: RowKey;
    output: RowDetail;
  };
  /** Add the rows of a CSV file to a collection */
  "collections.import_csv": {
    method: "POST";
    path: "/v1/collections/{collection_id}/csv";
    input: ImportCsvInput;
    output: CsvImported;
  };
  /** List the collections you can read */
  "collections.list": {
    method: "GET";
    path: "/v1/collections";
    input: ListCollections;
    output: CollectionList;
  };
  /** Run a view: filter, sort, group and compute rows */
  "collections.query": {
    method: "GET";
    path: "/v1/collections/{collection_id}/query";
    input: QueryInput;
    output: ViewResult;
  };
  /** Save a view: its layout, filters, sorts, grouping and hidden columns */
  "collections.save_view": {
    method: "PUT";
    path: "/v1/collections/{collection_id}/views/{view}";
    input: SaveViewInput;
    output: CollectionInfo;
  };
  /** Change a row's values or body */
  "collections.update_row": {
    method: "PATCH";
    path: "/v1/collections/{collection_id}/rows/{row_id}";
    input: UpdateRowInput;
    output: CollectionsUpdateRowRowWritten;
  };
  /** List the changes to an item, or to a whole space by its ID */
  "drive.activity": {
    method: "GET";
    path: "/v1/drive/items/{id}/activity";
    input: ActivityInput;
    output: ActivityList;
  };
  /** List the pages that link to a page */
  "drive.backlinks": {
    method: "GET";
    path: "/v1/drive/items/{id}/backlinks";
    input: NodeKey;
    output: NodeList;
  };
  /** Copy an item, with fresh IDs, into a folder or space */
  "drive.copy": {
    method: "POST";
    path: "/v1/drive/items/{id}/copy";
    input: PlaceInput;
    output: DriveCopyNodeView;
  };
  /** Create a folder */
  "drive.create_folder": {
    method: "POST";
    path: "/v1/drive/spaces/{space_id}/folders";
    input: CreateFolder;
    output: DriveCreateFolderNodeView;
  };
  /** Create a page */
  "drive.create_page": {
    method: "POST";
    path: "/v1/drive/spaces/{space_id}/pages";
    input: CreatePage;
    output: DriveCreatePageNodeView;
  };
  /** Create a shortcut to another item */
  "drive.create_shortcut": {
    method: "POST";
    path: "/v1/drive/spaces/{space_id}/shortcuts";
    input: CreateShortcut;
    output: DriveCreateShortcutNodeView;
  };
  /** Create a shared drive, or your My Drive */
  "drive.create_space": {
    method: "POST";
    path: "/v1/drive/spaces";
    input: CreateSpace;
    output: DriveCreateSpaceSpaceView;
  };
  /** Delete everything in a space's trash for good */
  "drive.empty_trash": {
    method: "DELETE";
    path: "/v1/drive/spaces/{space_id}/trash";
    input: SpaceKey;
    output: Removed;
  };
  /** Find items by title, creator, date, kind, place or star */
  "drive.find": {
    method: "GET";
    path: "/v1/drive/search";
    input: FindInput;
    output: NodeList;
  };
  /** Get an item */
  "drive.get": {
    method: "GET";
    path: "/v1/drive/items/{id}";
    input: NodeKey;
    output: DriveGetNodeView;
  };
  /** List the items in a folder or at the top of a space */
  "drive.list": {
    method: "GET";
    path: "/v1/drive/spaces/{space_id}/items";
    input: ListChildren;
    output: NodeList;
  };
  /** Move an item to another folder or space */
  "drive.move": {
    method: "POST";
    path: "/v1/drive/items/{id}/move";
    input: PlaceInput;
    output: DriveMoveNodeView;
  };
  /** Get your My Drive, made for you if you have none yet */
  "drive.my_drive": {
    method: "GET";
    path: "/v1/drive/my-drive";
    input: NoInput;
    output: MyDrive;
  };
  /** Apply the space's version retention now */
  "drive.prune_versions": {
    method: "POST";
    path: "/v1/drive/spaces/{space_id}/prune-versions";
    input: SpaceKey;
    output: Removed;
  };
  /** Delete a trashed item for good */
  "drive.purge": {
    method: "DELETE";
    path: "/v1/drive/items/{id}";
    input: NodeKey;
    output: DrivePurgeDone;
  };
  /** Read a page's content and its etag */
  "drive.read": {
    method: "GET";
    path: "/v1/drive/items/{id}/content";
    input: NodeKey;
    output: ContentView;
  };
  /** List the items you opened recently */
  "drive.recent": {
    method: "GET";
    path: "/v1/drive/recent";
    input: Limit;
    output: RecentList;
  };
  /** Rename an item and fix the links to it */
  "drive.rename": {
    method: "PATCH";
    path: "/v1/drive/items/{id}";
    input: RenameInput;
    output: DriveRenameNodeView;
  };
  /** Get the item a shortcut points at */
  "drive.resolve_shortcut": {
    method: "GET";
    path: "/v1/drive/items/{id}/target";
    input: NodeKey;
    output: DriveResolveShortcutNodeView;
  };
  /** Bring a trashed item back */
  "drive.restore": {
    method: "POST";
    path: "/v1/drive/items/{id}/restore";
    input: NodeKey;
    output: DriveRestoreNodeView;
  };
  /** Make an old version the page's content again */
  "drive.restore_version": {
    method: "POST";
    path: "/v1/drive/items/{id}/versions/{version_id}/restore";
    input: VersionKey;
    output: SaveView;
  };
  /** Save a page, merging changes made meanwhile */
  "drive.save": {
    method: "PUT";
    path: "/v1/drive/items/{id}/content";
    input: SaveInput;
    output: SaveView;
  };
  /** List the shared drives you can open */
  "drive.shared_drives": {
    method: "GET";
    path: "/v1/drive/shared-drives";
    input: NoInput;
    output: SpaceList;
  };
  /** List items in other people's My Drives shared with you */
  "drive.shared_with_me": {
    method: "GET";
    path: "/v1/drive/shared-with-me";
    input: NoInput;
    output: NodeList;
  };
  /** Star or unstar an item */
  "drive.star": {
    method: "PUT";
    path: "/v1/drive/items/{id}/star";
    input: StarInput;
    output: DriveStarDone;
  };
  /** List the items you starred */
  "drive.starred": {
    method: "GET";
    path: "/v1/drive/starred";
    input: NoInput;
    output: NodeList;
  };
  /** Move an item, with everything in it, to the trash */
  "drive.trash": {
    method: "POST";
    path: "/v1/drive/items/{id}/trash";
    input: NodeKey;
    output: DriveTrashNodeView;
  };
  /** List what was trashed in a space */
  "drive.trash_list": {
    method: "GET";
    path: "/v1/drive/spaces/{space_id}/trash";
    input: SpaceKey;
    output: NodeList;
  };
  /** List every item in a space, or under one item, in tree order */
  "drive.tree": {
    method: "GET";
    path: "/v1/drive/spaces/{space_id}/tree";
    input: TreeInput;
    output: TreeView;
  };
  /** Rename a space and change its quota, trash and version settings */
  "drive.update_space": {
    method: "PATCH";
    path: "/v1/drive/spaces/{space_id}";
    input: UpdateSpace;
    output: DriveUpdateSpaceSpaceView;
  };
  /** Get how much of its quota a space uses */
  "drive.usage": {
    method: "GET";
    path: "/v1/drive/spaces/{space_id}/usage";
    input: SpaceKey;
    output: UsageView;
  };
  /** Read one saved version of a page */
  "drive.version": {
    method: "GET";
    path: "/v1/drive/items/{id}/versions/{version_id}";
    input: VersionKey;
    output: VersionContent;
  };
  /** List a page's saved versions */
  "drive.versions": {
    method: "GET";
    path: "/v1/drive/items/{id}/versions";
    input: NodeKey;
    output: VersionList;
  };
  /** Delete a sealed page */
  "e2e.delete": {
    method: "DELETE";
    path: "/v1/spaces/{space_id}/e2e/object";
    input: DeleteObject;
    output: Deleted;
  };
  /** Enrol this device in a private space */
  "e2e.enrol": {
    method: "POST";
    path: "/v1/spaces/{space_id}/e2e/keys";
    input: EnrolDevice;
    output: E2eEnrolDeviceKeyInfo;
  };
  /** Download a sealed page */
  "e2e.get": {
    method: "GET";
    path: "/v1/spaces/{space_id}/e2e/object";
    input: GetObject;
    output: SealedObject;
  };
  /** List the devices a private space is sealed to */
  "e2e.keys": {
    method: "GET";
    path: "/v1/spaces/{space_id}/e2e/keys";
    input: SpaceRef;
    output: SpaceKeys;
  };
  /** List a private space's objects */
  "e2e.objects": {
    method: "GET";
    path: "/v1/spaces/{space_id}/e2e/objects";
    input: ListObjects;
    output: ObjectList;
  };
  /** Upload a page sealed on this device */
  "e2e.put": {
    method: "PUT";
    path: "/v1/spaces/{space_id}/e2e/object";
    input: PutObject;
    output: Stored;
  };
  /** Revoke a device from a private space */
  "e2e.revoke": {
    method: "DELETE";
    path: "/v1/spaces/{space_id}/e2e/keys/{fingerprint}";
    input: RevokeDevice;
    output: Revoked;
  };
  /** List apps you connected with OAuth */
  "identity.apps": {
    method: "GET";
    path: "/v1/me/apps";
    input: Empty;
    output: Array_of_AppView;
  };
  /** Create an API token; the secret is returned once */
  "identity.create_token": {
    method: "POST";
    path: "/v1/me/tokens";
    input: CreateToken;
    output: CreatedToken;
  };
  /** Create a user */
  "identity.create_user": {
    method: "POST";
    path: "/v1/admin/users";
    input: CreateUser;
    output: IdentityCreateUserUserView;
  };
  /** Disconnect an app */
  "identity.revoke_app": {
    method: "DELETE";
    path: "/v1/me/apps/{id}";
    input: ById;
    output: Done;
  };
  /** Sign out one of your browsers */
  "identity.revoke_session": {
    method: "DELETE";
    path: "/v1/me/sessions/{id}";
    input: ById;
    output: Done;
  };
  /** Revoke one of your API tokens */
  "identity.revoke_token": {
    method: "DELETE";
    path: "/v1/me/tokens/{id}";
    input: ById;
    output: Done;
  };
  /** List your signed-in browsers */
  "identity.sessions": {
    method: "GET";
    path: "/v1/me/sessions";
    input: Empty;
    output: Array_of_SessionView;
  };
  /** Create the first administrator of a new workspace */
  "identity.setup": {
    method: "POST";
    path: "/v1/setup";
    input: SetupInput;
    output: IdentitySetupUserView;
  };
  /** Who is signed in, and whether the workspace needs set-up */
  "identity.status": {
    method: "GET";
    path: "/v1/auth/status";
    input: Empty;
    output: AuthStatus;
  };
  /** List your API tokens */
  "identity.tokens": {
    method: "GET";
    path: "/v1/me/tokens";
    input: Empty;
    output: Array_of_TokenView;
  };
  /** Change your display name */
  "identity.update_me": {
    method: "PATCH";
    path: "/v1/me";
    input: UpdateMe;
    output: IdentityUpdateMeUserView;
  };
  /** Rename, promote, demote, block or unblock a user */
  "identity.update_user": {
    method: "PATCH";
    path: "/v1/admin/users/{id}";
    input: UpdateUser;
    output: IdentityUpdateUserUserView;
  };
  /** List every user */
  "identity.users": {
    method: "GET";
    path: "/v1/admin/users";
    input: Empty;
    output: Array_of_UserView;
  };
  /** Your notifications, newest first */
  "notify.inbox": {
    method: "GET";
    path: "/v1/notifications";
    input: InboxQuery;
    output: InboxPage;
  };
  /** Your notification channels, mutes and snooze */
  "notify.preferences": {
    method: "GET";
    path: "/v1/notifications/preferences";
    input: NotifyPreferencesEmpty;
    output: PreferencesView;
  };
  /** The server's web push key, for subscribing a browser */
  "notify.push_key": {
    method: "GET";
    path: "/v1/push/vapid-key";
    input: NotifyPushKeyEmpty;
    output: PushKey;
  };
  /** Register this browser for push notifications */
  "notify.push_subscribe": {
    method: "POST";
    path: "/v1/push/subscriptions";
    input: Subscribe;
    output: PushDevices;
  };
  /** Stop push notifications to one browser */
  "notify.push_unsubscribe": {
    method: "DELETE";
    path: "/v1/push/subscriptions";
    input: Unsubscribe;
    output: PushDevices;
  };
  /** Mark notifications read */
  "notify.read": {
    method: "POST";
    path: "/v1/notifications/read";
    input: MarkRead;
    output: Unread;
  };
  /** Change your notification channels, mutes and snooze */
  "notify.set_preferences": {
    method: "PUT";
    path: "/v1/notifications/preferences";
    input: PreferencesView;
    output: PreferencesView;
  };
  /** Renders a page as sanitised HTML for the reader */
  "render.page": {
    method: "GET";
    path: "/v1/pages/{id}/render";
    input: PageKey;
    output: RenderedPageView;
  };
  /** A block with the heading section around it, for quoting and citations */
  "search.context": {
    method: "GET";
    path: "/v1/search/context/{doc_id}/{block_id}";
    input: ContextInput;
    output: Context;
  };
  /** Searches blocks across everything the caller can read */
  "search.query": {
    method: "GET";
    path: "/v1/search";
    input: SearchQueryQueryInput;
    output: SearchPage;
  };
  /** Instant title search for the command menu */
  "search.titles": {
    method: "GET";
    path: "/v1/search/titles";
    input: TitlesInput;
    output: TitlesPage;
  };
  /** Completes a task; a repeating task moves to its next date */
  "tasks.complete": {
    method: "POST";
    path: "/v1/tasks/{task}/complete";
    input: CompleteInput;
    output: CompleteView;
  };
  /** Adds a task from a quick-add line */
  "tasks.create": {
    method: "POST";
    path: "/v1/tasks";
    input: CreateInput;
    output: TasksCreateTaskView;
  };
  /** Creates a task list */
  "tasks.create_list": {
    method: "POST";
    path: "/v1/task-lists";
    input: CreateListInput;
    output: TasksCreateListListView;
  };
  /** Removes a task and its subtasks */
  "tasks.delete": {
    method: "DELETE";
    path: "/v1/tasks/{task}";
    input: TaskRef;
    output: DeletedView;
  };
  /** Today's digest as Vellum Markdown */
  "tasks.digest": {
    method: "GET";
    path: "/v1/my-work/digest";
    input: TasksDigestNoInput;
    output: DigestView;
  };
  /** Starts connecting Google Calendar; returns Google's consent page */
  "tasks.google_connect": {
    method: "POST";
    path: "/v1/my-work/google/connect";
    input: TasksGoogleConnectNoInput;
    output: SecretView;
  };
  /** Disconnects Google Calendar */
  "tasks.google_disconnect": {
    method: "DELETE";
    path: "/v1/my-work/google";
    input: TasksGoogleDisconnectNoInput;
    output: TasksGoogleDisconnectDone;
  };
  /** Whether Google Calendar time-blocking is set up and connected */
  "tasks.google_status": {
    method: "GET";
    path: "/v1/my-work/google";
    input: TasksGoogleStatusNoInput;
    output: GoogleView;
  };
  /** Syncs time blocks with Google Calendar now */
  "tasks.google_sync": {
    method: "POST";
    path: "/v1/my-work/google/sync";
    input: TasksGoogleSyncNoInput;
    output: SyncView;
  };
  /** Imports tasks from Google Tasks, Todoist or Things */
  "tasks.import": {
    method: "POST";
    path: "/v1/tasks/import";
    input: ImportInput;
    output: ImportView;
  };
  /** Lists the person's task lists and lists shared with them */
  "tasks.lists": {
    method: "GET";
    path: "/v1/task-lists";
    input: TasksListsNoInput;
    output: ListsView;
  };
  /** Takes a mention out of My Work without replying */
  "tasks.mention_dismiss": {
    method: "POST";
    path: "/v1/my-work/mentions/{comment}/dismiss";
    input: MentionRef;
    output: TasksMentionDismissDone;
  };
  /** Moves a task to another bucket or day */
  "tasks.move": {
    method: "POST";
    path: "/v1/tasks/{task}/move";
    input: MoveInput;
    output: TasksMoveTaskView;
  };
  /** Lists My Work: Inbox, Today, Evening, Upcoming, Anytime, Someday, Done */
  "tasks.my_work": {
    method: "GET";
    path: "/v1/my-work";
    input: TasksMyWorkNoInput;
    output: MyWorkView;
  };
  /** Parses a quick-add line for highlighting */
  "tasks.parse": {
    method: "GET";
    path: "/v1/tasks/parse";
    input: ParseInput;
    output: ParseView;
  };
  /** Reopens a completed task */
  "tasks.reopen": {
    method: "POST";
    path: "/v1/tasks/{task}/reopen";
    input: TaskRef;
    output: TasksReopenTaskView;
  };
  /** Turns capture by e-mail off */
  "tasks.revoke_capture": {
    method: "DELETE";
    path: "/v1/my-work/capture-address";
    input: TasksRevokeCaptureNoInput;
    output: TasksRevokeCaptureDone;
  };
  /** Turns the calendar feed off */
  "tasks.revoke_feed": {
    method: "DELETE";
    path: "/v1/my-work/feed";
    input: TasksRevokeFeedNoInput;
    output: TasksRevokeFeedDone;
  };
  /** Issues a new capture e-mail address and revokes the old one */
  "tasks.rotate_capture": {
    method: "POST";
    path: "/v1/my-work/capture-address";
    input: TasksRotateCaptureNoInput;
    output: SecretView;
  };
  /** Issues a new calendar feed URL and revokes the old one */
  "tasks.rotate_feed": {
    method: "POST";
    path: "/v1/my-work/feed";
    input: TasksRotateFeedNoInput;
    output: SecretView;
  };
  /** Your My Work settings: time zone, start of day and digest time */
  "tasks.settings": {
    method: "GET";
    path: "/v1/my-work/settings";
    input: TasksSettingsNoInput;
    output: MyWorkSettingsView;
  };
  /** Replaces your My Work settings */
  "tasks.settings_update": {
    method: "PUT";
    path: "/v1/my-work/settings";
    input: MyWorkSettingsInput;
    output: MyWorkSettingsView;
  };
  /** Shares a list with someone */
  "tasks.share_list": {
    method: "POST";
    path: "/v1/task-lists/{list_id}/members";
    input: MemberInput;
    output: TasksShareListListView;
  };
  /** Stops sharing a list with someone */
  "tasks.unshare_list": {
    method: "DELETE";
    path: "/v1/task-lists/{list_id}/members/{member}";
    input: MemberInput;
    output: TasksUnshareListListView;
  };
  /** Edits a task's title or fields */
  "tasks.update": {
    method: "PATCH";
    path: "/v1/tasks/{task}";
    input: UpdateInput;
    output: TasksUpdateTaskView;
  };
  /** Applies the waiting triage suggestion as the caller */
  "work.accept_suggestion": {
    method: "POST";
    path: "/v1/work/issues/{key}/triage/suggestion/accept";
    input: IssueRef;
    output: WorkAcceptSuggestionIssueOut;
  };
  /** Starts a thread or replies */
  "work.add_comment": {
    method: "POST";
    path: "/v1/work/comments";
    input: AddComment;
    output: CommentResult;
  };
  /** Assigns, reassigns or unassigns a thread */
  "work.assign_thread": {
    method: "POST";
    path: "/v1/work/threads/{thread_id}/assign";
    input: AssignThread;
    output: CommentResult;
  };
  /** Creates an issue */
  "work.create_issue": {
    method: "POST";
    path: "/v1/work/issues";
    input: CreateIssue;
    output: CreatedIssue;
  };
  /** Creates a team */
  "work.create_team": {
    method: "POST";
    path: "/v1/work/teams";
    input: CreateTeam;
    output: WorkCreateTeamTeamOut;
  };
  /** Deletes my comment, or any comment as an admin */
  "work.delete_comment": {
    method: "DELETE";
    path: "/v1/work/comments/{comment_id}";
    input: CommentRef;
    output: CommentResult;
  };
  /** Drops the waiting triage suggestion */
  "work.dismiss_suggestion": {
    method: "DELETE";
    path: "/v1/work/issues/{key}/triage/suggestion";
    input: IssueRef;
    output: WorkDismissSuggestionIssueOut;
  };
  /** Edits my comment */
  "work.edit_comment": {
    method: "PATCH";
    path: "/v1/work/comments/{comment_id}";
    input: EditComment;
    output: CommentResult;
  };
  /** Gets an issue by key, old key or ID */
  "work.get_issue": {
    method: "GET";
    path: "/v1/work/issues/{key}";
    input: IssueRef;
    output: WorkGetIssueIssueOut;
  };
  /** Gets a project with progress */
  "work.get_project": {
    method: "GET";
    path: "/v1/work/projects/{project_id}";
    input: ProjectRef;
    output: WorkGetProjectProjectOut;
  };
  /** Gets a team by key */
  "work.get_team": {
    method: "GET";
    path: "/v1/work/teams/{team}";
    input: TeamRef;
    output: WorkGetTeamTeamOut;
  };
  /** Lists a team's triage inbox */
  "work.inbox": {
    method: "GET";
    path: "/v1/work/teams/{team}/inbox";
    input: TeamRef;
    output: IssueGroups;
  };
  /** Measures lead, cycle and triage time, throughput and intake */
  "work.insights": {
    method: "GET";
    path: "/v1/work/insights";
    input: InsightsQuery;
    output: InsightsOut;
  };
  /** Lists comment threads on an issue, page or block */
  "work.list_comments": {
    method: "GET";
    path: "/v1/work/comments";
    input: ListComments;
    output: ThreadsOut;
  };
  /** Lists a team's cycles with burn-up and velocity */
  "work.list_cycles": {
    method: "GET";
    path: "/v1/work/teams/{team}/cycles";
    input: TeamRef;
    output: CyclesOut;
  };
  /** Lists a team's intake forms */
  "work.list_forms": {
    method: "GET";
    path: "/v1/work/teams/{team}/forms";
    input: TeamRef;
    output: FormsOut;
  };
  /** Lists projects with progress */
  "work.list_projects": {
    method: "GET";
    path: "/v1/work/projects";
    input: WorkListProjectsNoInput;
    output: ProjectsOut;
  };
  /** Lists teams with their workflows */
  "work.list_teams": {
    method: "GET";
    path: "/v1/work/teams";
    input: WorkListTeamsNoInput;
    output: TeamsOut;
  };
  /** Moves an issue to another team; the old key keeps working */
  "work.move_issue": {
    method: "POST";
    path: "/v1/work/issues/{key}/move";
    input: MoveIssue;
    output: WorkMoveIssueIssueOut;
  };
  /** Lists comment threads assigned to me */
  "work.my_assignments": {
    method: "GET";
    path: "/v1/work/me/assignments";
    input: WorkMyAssignmentsNoInput;
    output: AssignmentsOut;
  };
  /** Lists my issues in focus order */
  "work.my_issues": {
    method: "GET";
    path: "/v1/work/me/issues";
    input: MyIssues;
    output: MyIssuesOut;
  };
  /** Creates or replaces a project page */
  "work.put_project": {
    method: "PUT";
    path: "/v1/work/projects/{project_id}";
    input: PutProject;
    output: WorkPutProjectProjectOut;
  };
  /** Replaces a team's configuration (team.yaml) */
  "work.put_team": {
    method: "PUT";
    path: "/v1/work/teams/{team}";
    input: PutTeam;
    output: WorkPutTeamTeamOut;
  };
  /** Lists issues by filter, grouped and ordered */
  "work.query_issues": {
    method: "GET";
    path: "/v1/work/issues";
    input: QueryIssues;
    output: IssueGroups;
  };
  /** Adds or removes my reaction */
  "work.react": {
    method: "POST";
    path: "/v1/work/comments/{comment_id}/reactions";
    input: React;
    output: CommentResult;
  };
  /** Resolves or reopens a thread */
  "work.resolve_thread": {
    method: "POST";
    path: "/v1/work/threads/{thread_id}/resolve";
    input: ResolveThread;
    output: CommentResult;
  };
  /** Submits answers to an intake form as a triage issue */
  "work.submit_form": {
    method: "POST";
    path: "/v1/work/teams/{team}/forms/{slug}";
    input: SubmitForm;
    output: WorkSubmitFormIssueOut;
  };
  /** Proposes a triage decision for a person to accept */
  "work.suggest_triage": {
    method: "PUT";
    path: "/v1/work/issues/{key}/triage/suggestion";
    input: SuggestTriage;
    output: WorkSuggestTriageSuggestionOut;
  };
  /** Gets what an assistant needs to suggest a triage decision */
  "work.triage_brief": {
    method: "GET";
    path: "/v1/work/issues/{key}/triage/brief";
    input: IssueRef;
    output: BriefOut;
  };
  /** Accepts, declines, marks duplicate or snoozes an issue in triage */
  "work.triage_issue": {
    method: "POST";
    path: "/v1/work/issues/{key}/triage";
    input: TriageIssue;
    output: WorkTriageIssueIssueOut;
  };
  /** Changes fields of an issue */
  "work.update_issue": {
    method: "PATCH";
    path: "/v1/work/issues/{key}";
    input: UpdateIssue;
    output: WorkUpdateIssueIssueOut;
  };
}

/** How to call each operation, for the runtime in `client.ts`. */
export const OPERATIONS = {
  "admin.appearance": { method: "GET", path: "/v1/appearance", mutation: false },
  "admin.client_report": { method: "POST", path: "/v1/telemetry/client", mutation: true },
  "admin.create_backup": { method: "POST", path: "/v1/admin/backups", mutation: true },
  "admin.export_audit": { method: "GET", path: "/v1/admin/audit/export", mutation: false },
  "admin.get_request": { method: "GET", path: "/v1/admin/requests/{trace_id}", mutation: false },
  "admin.get_settings": { method: "GET", path: "/v1/admin/settings", mutation: false },
  "admin.list_audit": { method: "GET", path: "/v1/admin/audit", mutation: false },
  "admin.list_backups": { method: "GET", path: "/v1/admin/backups", mutation: false },
  "admin.list_requests": { method: "GET", path: "/v1/admin/requests", mutation: false },
  "admin.profile": { method: "GET", path: "/v1/admin/profile", mutation: false },
  "admin.routes": { method: "GET", path: "/v1/admin/routes", mutation: false },
  "admin.run_drill": { method: "POST", path: "/v1/admin/backups/drill", mutation: true },
  "admin.slos": { method: "GET", path: "/v1/admin/slos", mutation: false },
  "admin.state": { method: "GET", path: "/v1/admin/state", mutation: false },
  "admin.telemetry_preview": { method: "GET", path: "/v1/admin/telemetry/preview", mutation: false },
  "admin.update_settings": { method: "PUT", path: "/v1/admin/settings", mutation: true },
  "admin.verify_audit": { method: "POST", path: "/v1/admin/audit/verify", mutation: true },
  "books.add_section": { method: "POST", path: "/v1/books/{book_id}/sections", mutation: true },
  "books.create_snapshot": { method: "POST", path: "/v1/books/{book_id}/snapshots", mutation: true },
  "books.diff": { method: "GET", path: "/v1/books/{book_id}/diff", mutation: false },
  "books.get": { method: "GET", path: "/v1/books/{book_id}", mutation: false },
  "books.get_section": { method: "GET", path: "/v1/books/{book_id}/sections/{section_id}", mutation: false },
  "books.list": { method: "GET", path: "/v1/books", mutation: false },
  "books.list_snapshots": { method: "GET", path: "/v1/books/{book_id}/snapshots", mutation: false },
  "books.move_section": { method: "POST", path: "/v1/books/{book_id}/sections/{section_id}/move", mutation: true },
  "books.remove_section": { method: "DELETE", path: "/v1/books/{book_id}/sections/{section_id}", mutation: true },
  "books.set_section_access": { method: "PUT", path: "/v1/books/{book_id}/sections/{section_id}/access", mutation: true },
  "books.update_section": { method: "PATCH", path: "/v1/books/{book_id}/sections/{section_id}", mutation: true },
  "collections.chart": { method: "GET", path: "/v1/collections/chart", mutation: false },
  "collections.create_from_csv": { method: "POST", path: "/v1/collections", mutation: true },
  "collections.create_row": { method: "POST", path: "/v1/collections/{collection_id}/rows", mutation: true },
  "collections.delete_row": { method: "DELETE", path: "/v1/collections/{collection_id}/rows/{row_id}", mutation: true },
  "collections.delete_view": { method: "DELETE", path: "/v1/collections/{collection_id}/views/{view}", mutation: true },
  "collections.embed": { method: "GET", path: "/v1/collections/embed", mutation: false },
  "collections.export_csv": { method: "GET", path: "/v1/collections/{collection_id}/csv", mutation: false },
  "collections.get": { method: "GET", path: "/v1/collections/{collection_id}", mutation: false },
  "collections.get_row": { method: "GET", path: "/v1/collections/{collection_id}/rows/{row_id}", mutation: false },
  "collections.import_csv": { method: "POST", path: "/v1/collections/{collection_id}/csv", mutation: true },
  "collections.list": { method: "GET", path: "/v1/collections", mutation: false },
  "collections.query": { method: "GET", path: "/v1/collections/{collection_id}/query", mutation: false },
  "collections.save_view": { method: "PUT", path: "/v1/collections/{collection_id}/views/{view}", mutation: true },
  "collections.update_row": { method: "PATCH", path: "/v1/collections/{collection_id}/rows/{row_id}", mutation: true },
  "drive.activity": { method: "GET", path: "/v1/drive/items/{id}/activity", mutation: false },
  "drive.backlinks": { method: "GET", path: "/v1/drive/items/{id}/backlinks", mutation: false },
  "drive.copy": { method: "POST", path: "/v1/drive/items/{id}/copy", mutation: true },
  "drive.create_folder": { method: "POST", path: "/v1/drive/spaces/{space_id}/folders", mutation: true },
  "drive.create_page": { method: "POST", path: "/v1/drive/spaces/{space_id}/pages", mutation: true },
  "drive.create_shortcut": { method: "POST", path: "/v1/drive/spaces/{space_id}/shortcuts", mutation: true },
  "drive.create_space": { method: "POST", path: "/v1/drive/spaces", mutation: true },
  "drive.empty_trash": { method: "DELETE", path: "/v1/drive/spaces/{space_id}/trash", mutation: true },
  "drive.find": { method: "GET", path: "/v1/drive/search", mutation: false },
  "drive.get": { method: "GET", path: "/v1/drive/items/{id}", mutation: false },
  "drive.list": { method: "GET", path: "/v1/drive/spaces/{space_id}/items", mutation: false },
  "drive.move": { method: "POST", path: "/v1/drive/items/{id}/move", mutation: true },
  "drive.my_drive": { method: "GET", path: "/v1/drive/my-drive", mutation: false },
  "drive.prune_versions": { method: "POST", path: "/v1/drive/spaces/{space_id}/prune-versions", mutation: true },
  "drive.purge": { method: "DELETE", path: "/v1/drive/items/{id}", mutation: true },
  "drive.read": { method: "GET", path: "/v1/drive/items/{id}/content", mutation: false },
  "drive.recent": { method: "GET", path: "/v1/drive/recent", mutation: false },
  "drive.rename": { method: "PATCH", path: "/v1/drive/items/{id}", mutation: true },
  "drive.resolve_shortcut": { method: "GET", path: "/v1/drive/items/{id}/target", mutation: false },
  "drive.restore": { method: "POST", path: "/v1/drive/items/{id}/restore", mutation: true },
  "drive.restore_version": { method: "POST", path: "/v1/drive/items/{id}/versions/{version_id}/restore", mutation: true },
  "drive.save": { method: "PUT", path: "/v1/drive/items/{id}/content", mutation: true },
  "drive.shared_drives": { method: "GET", path: "/v1/drive/shared-drives", mutation: false },
  "drive.shared_with_me": { method: "GET", path: "/v1/drive/shared-with-me", mutation: false },
  "drive.star": { method: "PUT", path: "/v1/drive/items/{id}/star", mutation: true },
  "drive.starred": { method: "GET", path: "/v1/drive/starred", mutation: false },
  "drive.trash": { method: "POST", path: "/v1/drive/items/{id}/trash", mutation: true },
  "drive.trash_list": { method: "GET", path: "/v1/drive/spaces/{space_id}/trash", mutation: false },
  "drive.tree": { method: "GET", path: "/v1/drive/spaces/{space_id}/tree", mutation: false },
  "drive.update_space": { method: "PATCH", path: "/v1/drive/spaces/{space_id}", mutation: true },
  "drive.usage": { method: "GET", path: "/v1/drive/spaces/{space_id}/usage", mutation: false },
  "drive.version": { method: "GET", path: "/v1/drive/items/{id}/versions/{version_id}", mutation: false },
  "drive.versions": { method: "GET", path: "/v1/drive/items/{id}/versions", mutation: false },
  "e2e.delete": { method: "DELETE", path: "/v1/spaces/{space_id}/e2e/object", mutation: true },
  "e2e.enrol": { method: "POST", path: "/v1/spaces/{space_id}/e2e/keys", mutation: true },
  "e2e.get": { method: "GET", path: "/v1/spaces/{space_id}/e2e/object", mutation: false },
  "e2e.keys": { method: "GET", path: "/v1/spaces/{space_id}/e2e/keys", mutation: false },
  "e2e.objects": { method: "GET", path: "/v1/spaces/{space_id}/e2e/objects", mutation: false },
  "e2e.put": { method: "PUT", path: "/v1/spaces/{space_id}/e2e/object", mutation: true },
  "e2e.revoke": { method: "DELETE", path: "/v1/spaces/{space_id}/e2e/keys/{fingerprint}", mutation: true },
  "identity.apps": { method: "GET", path: "/v1/me/apps", mutation: false },
  "identity.create_token": { method: "POST", path: "/v1/me/tokens", mutation: true },
  "identity.create_user": { method: "POST", path: "/v1/admin/users", mutation: true },
  "identity.revoke_app": { method: "DELETE", path: "/v1/me/apps/{id}", mutation: true },
  "identity.revoke_session": { method: "DELETE", path: "/v1/me/sessions/{id}", mutation: true },
  "identity.revoke_token": { method: "DELETE", path: "/v1/me/tokens/{id}", mutation: true },
  "identity.sessions": { method: "GET", path: "/v1/me/sessions", mutation: false },
  "identity.setup": { method: "POST", path: "/v1/setup", mutation: true },
  "identity.status": { method: "GET", path: "/v1/auth/status", mutation: false },
  "identity.tokens": { method: "GET", path: "/v1/me/tokens", mutation: false },
  "identity.update_me": { method: "PATCH", path: "/v1/me", mutation: true },
  "identity.update_user": { method: "PATCH", path: "/v1/admin/users/{id}", mutation: true },
  "identity.users": { method: "GET", path: "/v1/admin/users", mutation: false },
  "notify.inbox": { method: "GET", path: "/v1/notifications", mutation: false },
  "notify.preferences": { method: "GET", path: "/v1/notifications/preferences", mutation: false },
  "notify.push_key": { method: "GET", path: "/v1/push/vapid-key", mutation: false },
  "notify.push_subscribe": { method: "POST", path: "/v1/push/subscriptions", mutation: true },
  "notify.push_unsubscribe": { method: "DELETE", path: "/v1/push/subscriptions", mutation: true },
  "notify.read": { method: "POST", path: "/v1/notifications/read", mutation: true },
  "notify.set_preferences": { method: "PUT", path: "/v1/notifications/preferences", mutation: true },
  "render.page": { method: "GET", path: "/v1/pages/{id}/render", mutation: false },
  "search.context": { method: "GET", path: "/v1/search/context/{doc_id}/{block_id}", mutation: false },
  "search.query": { method: "GET", path: "/v1/search", mutation: false },
  "search.titles": { method: "GET", path: "/v1/search/titles", mutation: false },
  "tasks.complete": { method: "POST", path: "/v1/tasks/{task}/complete", mutation: true },
  "tasks.create": { method: "POST", path: "/v1/tasks", mutation: true },
  "tasks.create_list": { method: "POST", path: "/v1/task-lists", mutation: true },
  "tasks.delete": { method: "DELETE", path: "/v1/tasks/{task}", mutation: true },
  "tasks.digest": { method: "GET", path: "/v1/my-work/digest", mutation: false },
  "tasks.google_connect": { method: "POST", path: "/v1/my-work/google/connect", mutation: true },
  "tasks.google_disconnect": { method: "DELETE", path: "/v1/my-work/google", mutation: true },
  "tasks.google_status": { method: "GET", path: "/v1/my-work/google", mutation: false },
  "tasks.google_sync": { method: "POST", path: "/v1/my-work/google/sync", mutation: true },
  "tasks.import": { method: "POST", path: "/v1/tasks/import", mutation: true },
  "tasks.lists": { method: "GET", path: "/v1/task-lists", mutation: false },
  "tasks.mention_dismiss": { method: "POST", path: "/v1/my-work/mentions/{comment}/dismiss", mutation: true },
  "tasks.move": { method: "POST", path: "/v1/tasks/{task}/move", mutation: true },
  "tasks.my_work": { method: "GET", path: "/v1/my-work", mutation: false },
  "tasks.parse": { method: "GET", path: "/v1/tasks/parse", mutation: false },
  "tasks.reopen": { method: "POST", path: "/v1/tasks/{task}/reopen", mutation: true },
  "tasks.revoke_capture": { method: "DELETE", path: "/v1/my-work/capture-address", mutation: true },
  "tasks.revoke_feed": { method: "DELETE", path: "/v1/my-work/feed", mutation: true },
  "tasks.rotate_capture": { method: "POST", path: "/v1/my-work/capture-address", mutation: true },
  "tasks.rotate_feed": { method: "POST", path: "/v1/my-work/feed", mutation: true },
  "tasks.settings": { method: "GET", path: "/v1/my-work/settings", mutation: false },
  "tasks.settings_update": { method: "PUT", path: "/v1/my-work/settings", mutation: true },
  "tasks.share_list": { method: "POST", path: "/v1/task-lists/{list_id}/members", mutation: true },
  "tasks.unshare_list": { method: "DELETE", path: "/v1/task-lists/{list_id}/members/{member}", mutation: true },
  "tasks.update": { method: "PATCH", path: "/v1/tasks/{task}", mutation: true },
  "work.accept_suggestion": { method: "POST", path: "/v1/work/issues/{key}/triage/suggestion/accept", mutation: true },
  "work.add_comment": { method: "POST", path: "/v1/work/comments", mutation: true },
  "work.assign_thread": { method: "POST", path: "/v1/work/threads/{thread_id}/assign", mutation: true },
  "work.create_issue": { method: "POST", path: "/v1/work/issues", mutation: true },
  "work.create_team": { method: "POST", path: "/v1/work/teams", mutation: true },
  "work.delete_comment": { method: "DELETE", path: "/v1/work/comments/{comment_id}", mutation: true },
  "work.dismiss_suggestion": { method: "DELETE", path: "/v1/work/issues/{key}/triage/suggestion", mutation: true },
  "work.edit_comment": { method: "PATCH", path: "/v1/work/comments/{comment_id}", mutation: true },
  "work.get_issue": { method: "GET", path: "/v1/work/issues/{key}", mutation: false },
  "work.get_project": { method: "GET", path: "/v1/work/projects/{project_id}", mutation: false },
  "work.get_team": { method: "GET", path: "/v1/work/teams/{team}", mutation: false },
  "work.inbox": { method: "GET", path: "/v1/work/teams/{team}/inbox", mutation: false },
  "work.insights": { method: "GET", path: "/v1/work/insights", mutation: false },
  "work.list_comments": { method: "GET", path: "/v1/work/comments", mutation: false },
  "work.list_cycles": { method: "GET", path: "/v1/work/teams/{team}/cycles", mutation: false },
  "work.list_forms": { method: "GET", path: "/v1/work/teams/{team}/forms", mutation: false },
  "work.list_projects": { method: "GET", path: "/v1/work/projects", mutation: false },
  "work.list_teams": { method: "GET", path: "/v1/work/teams", mutation: false },
  "work.move_issue": { method: "POST", path: "/v1/work/issues/{key}/move", mutation: true },
  "work.my_assignments": { method: "GET", path: "/v1/work/me/assignments", mutation: false },
  "work.my_issues": { method: "GET", path: "/v1/work/me/issues", mutation: false },
  "work.put_project": { method: "PUT", path: "/v1/work/projects/{project_id}", mutation: true },
  "work.put_team": { method: "PUT", path: "/v1/work/teams/{team}", mutation: true },
  "work.query_issues": { method: "GET", path: "/v1/work/issues", mutation: false },
  "work.react": { method: "POST", path: "/v1/work/comments/{comment_id}/reactions", mutation: true },
  "work.resolve_thread": { method: "POST", path: "/v1/work/threads/{thread_id}/resolve", mutation: true },
  "work.submit_form": { method: "POST", path: "/v1/work/teams/{team}/forms/{slug}", mutation: true },
  "work.suggest_triage": { method: "PUT", path: "/v1/work/issues/{key}/triage/suggestion", mutation: true },
  "work.triage_brief": { method: "GET", path: "/v1/work/issues/{key}/triage/brief", mutation: false },
  "work.triage_issue": { method: "POST", path: "/v1/work/issues/{key}/triage", mutation: true },
  "work.update_issue": { method: "PATCH", path: "/v1/work/issues/{key}", mutation: true },
} as const;
