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
 * How rollups combine the related rows' values.
 */
export type Aggregation = "count" | "sum" | "avg" | "min" | "max" | "checked" | "pct" | "unique" | "show";

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
   * The signed-in user, if any.
   */
  user?: UserView | null;
}

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
 * A DocId as a 26-character ULID.
 */
export type DocId = string;

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
   * The space; absent until it is created.
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
 * How a numeric result is displayed.
 */
export type NumberFormat = "plain" | "percent";

/**
 * A ObjectId as a 26-character ULID.
 */
export type ObjectId = string;

/**
 * Where to move or copy an item.
 */
export interface PlaceInput {
  /**
   * The item.
   */
  id: ObjectId;
  /**
   * The folder it goes into; the top of the space when absent.
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
  /** Get your My Drive */
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
  "drive.update_space": { method: "PATCH", path: "/v1/drive/spaces/{space_id}", mutation: true },
  "drive.usage": { method: "GET", path: "/v1/drive/spaces/{space_id}/usage", mutation: false },
  "drive.version": { method: "GET", path: "/v1/drive/items/{id}/versions/{version_id}", mutation: false },
  "drive.versions": { method: "GET", path: "/v1/drive/items/{id}/versions", mutation: false },
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
