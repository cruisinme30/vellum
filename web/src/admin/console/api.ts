// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The admin console's operations (`/api/v1/admin`): audit, settings and
 * backups (issue #254), users and spaces (issue #175), over the app's HTTP
 * client so writes carry the CSRF token and an old sign-in is asked to
 * confirm again (D241). Types come from the server's generated client.
 */

import type {
  AdminCreateSpace,
  AdminTokenView,
  AdminSettings,
  AdminSpaceList,
  AdminSpaceView,
  AdminState,
  AuditList,
  AuditQuery,
  AuditRecord,
  BackupFile,
  BackupInfo,
  ChainReport,
  CreateUser,
  DrillReport,
  GroupList,
  GroupMembersView,
  GroupView,
  ProviderInput,
  ProviderView,
  SamlMetadataView,
  ScimTokenCreated,
  UsageCounts,
  SearchStatusView,
  SettingsView,
  SignInView,
  SpaceRoleView,
  TokenPolicy,
  UpdateSignIn,
  UserView,
  Versioned,
} from "../../api/api.gen";
import { ApiError, type Http, type Query } from "../../api/http";

import { createHttpWebhooksApi, type WebhooksApi } from "./webhooks-api";

/** The admin console's server operations. */
export interface AdminConsoleApi {
  /** Audit records, newest first. */
  audit(query: AuditQuery): Promise<AuditRecord[]>;
  /** Checks the audit log's hash chain end to end. */
  verifyAudit(): Promise<ChainReport>;
  /** The whole audit log as JSON Lines (the export itself is audited). */
  exportAudit(): Promise<string>;
  settings(): Promise<Versioned>;
  /** Saves, unless someone saved since `expectedVersion` (a conflict error). */
  saveSettings(expectedVersion: number, settings: AdminSettings): Promise<Versioned>;
  state(): Promise<AdminState>;
  backups(): Promise<BackupFile[]>;
  backUpNow(): Promise<BackupInfo>;
  runDrill(): Promise<DrillReport>;
  /** People, content, storage and activity; `refresh` counts again now. */
  usage(refresh: boolean): Promise<UsageCounts>;
  /** Search's engine, embedding model and background indexing (D300). */
  readonly searchStatus?: () => Promise<SearchStatusView>;
  /** Webhook subscriptions (#149). */
  readonly webhooks?: WebhooksApi;
  /** Every account, oldest first. */
  users(): Promise<UserView[]>;
  createUser(input: CreateUser): Promise<UserView>;
  /** Renames, promotes, demotes, blocks or unblocks; blocking ends every session. */
  updateUser(id: string, change: UserChange): Promise<UserView>;
  /** Every space with its size and members, shared drives first. */
  spaces(): Promise<AdminSpaceView[]>;
  createSpace(input: AdminCreateSpace): Promise<AdminSpaceView>;
  updateSpace(spaceId: string, change: SpaceChange): Promise<AdminSpaceView>;
  /** Gives someone exactly `role` on a shared drive; no role removes them. */
  setMember(
    spaceId: string,
    userId: string,
    role: SpaceRoleView | undefined,
  ): Promise<AdminSpaceView>;
  /** Every named group, by name. */
  groups(): Promise<GroupView[]>;
  createGroup(name: string, description: string): Promise<GroupView>;
  updateGroup(id: string, change: { name?: string; description?: string }): Promise<GroupView>;
  /** Deletes a group; its members lose what it granted. */
  deleteGroup(id: string): Promise<void>;
  groupMembers(id: string): Promise<GroupMembersView>;
  /** Everyone's personal API tokens (issue #523). */
  readonly tokens?: AdminTokensApi;
  /** Sign-in rules and per-person sign-in help (issue #523). */
  readonly signIn?: AdminSignInApi;
  changeGroupMembers(
    id: string,
    change: { add?: readonly string[]; remove?: readonly string[] },
  ): Promise<GroupMembersView>;
}

/** Everyone's personal API tokens, for admins (issue #523, ADR 0069). */
export interface AdminTokensApi {
  /** Every token, newest first, with its owner; never the secrets. */
  list(): Promise<AdminTokenView[]>;
  revoke(id: string): Promise<void>;
  /** The current cap on new tokens' expiry. */
  policy(): Promise<TokenPolicy>;
  /** Caps the expiry of new tokens; `undefined` lifts the cap. */
  setPolicy(maxDays: number | undefined): Promise<TokenPolicy>;
}

/** The workspace's sign-in rules, and helping one person sign in (issue #523, ADR 0069). */
export interface AdminSignInApi {
  settings(): Promise<SignInView>;
  update(change: UpdateSignIn): Promise<SignInView>;
  /** Removes someone's authenticator app, passkeys and recovery codes; ends their sessions. */
  resetFactors(userId: string): Promise<UserView>;
  /** Ends every session of one person. */
  signOutEverywhere(userId: string): Promise<number>;
  /** Adds an OpenID Connect provider (ADR 0069, D292). */
  addProvider(input: ProviderInput): Promise<ProviderView>;
  /** Changes a provider; an absent client secret keeps the stored one. */
  updateProvider(id: string, input: ProviderInput): Promise<ProviderView>;
  /** Removes a provider and answers with the settings as they now are. */
  removeProvider(id: string): Promise<SignInView>;
  /** Reads a SAML provider's metadata URL to fill in its settings (D293). */
  samlMetadata(url: string): Promise<SamlMetadataView>;
  /** Makes a new SCIM token, shown once; the old one stops working (D294). */
  rotateScim(): Promise<ScimTokenCreated>;
  /** Turns SCIM provisioning off. */
  revokeScim(): Promise<SignInView>;
}

/** What an admin changes about an account; unset fields stay as they are. */
export interface UserChange {
  readonly name?: string;
  readonly admin?: boolean;
  readonly disabled?: boolean;
  /** Lets an admin keep using a password while single sign-on is required. */
  readonly breakGlass?: boolean;
}

/** A space's new name (unchanged when absent) and its full settings. */
interface SpaceChange {
  readonly name?: string;
  readonly settings: SettingsView;
}

const ADMIN = "/api/v1/admin";

/** Drops unset filters, so the query string holds only what was asked. */
function auditQuery(query: AuditQuery): Query {
  const out: Record<string, string | number> = {};
  if (query.action) out["action"] = query.action;
  if (query.user_id) out["user_id"] = query.user_id;
  if (typeof query.before === "number") out["before"] = query.before;
  if (typeof query.limit === "number") out["limit"] = query.limit;
  return out;
}

/** The admin console over a Vellum server. */
export function createHttpAdminConsoleApi(http: Http): AdminConsoleApi {
  return {
    async audit(query) {
      const list = await http.get<AuditList>(`${ADMIN}/audit`, auditQuery(query));
      return list.records;
    },
    verifyAudit: () => http.send<ChainReport>("POST", `${ADMIN}/audit/verify`),
    async exportAudit() {
      const exported = await http.get<{ jsonl: string }>(`${ADMIN}/audit/export`);
      return exported.jsonl;
    },
    settings: () => http.get<Versioned>(`${ADMIN}/settings`),
    saveSettings: (expectedVersion, settings) =>
      http.send<Versioned>("PUT", `${ADMIN}/settings`, {
        expected_version: expectedVersion,
        settings,
      }),
    state: () => http.get<AdminState>(`${ADMIN}/state`),
    async backups() {
      const list = await http.get<{ backups: BackupFile[] }>(`${ADMIN}/backups`);
      return list.backups;
    },
    backUpNow: () => http.send<BackupInfo>("POST", `${ADMIN}/backups`),
    runDrill: () => http.send<DrillReport>("POST", `${ADMIN}/backups/drill`),
    usage: (refresh) => http.get<UsageCounts>(`${ADMIN}/usage`, refresh ? { refresh: "true" } : {}),
    searchStatus: () => http.get<SearchStatusView>(`${ADMIN}/search`),
    webhooks: createHttpWebhooksApi(http),
    users: () => http.get<UserView[]>(`${ADMIN}/users`),
    createUser: (input) => http.send<UserView>("POST", `${ADMIN}/users`, input),
    updateUser: (id, change) =>
      http.send<UserView>("PATCH", `${ADMIN}/users/${encodeURIComponent(id)}`, change),
    async spaces() {
      const list = await http.get<AdminSpaceList>(`${ADMIN}/spaces`);
      return list.spaces;
    },
    createSpace: (input) => http.send<AdminSpaceView>("POST", `${ADMIN}/spaces`, input),
    updateSpace: (spaceId, change) =>
      http.send<AdminSpaceView>("PATCH", `${ADMIN}/spaces/${encodeURIComponent(spaceId)}`, change),
    setMember: (spaceId, userId, role) =>
      http.send<AdminSpaceView>(
        "PUT",
        `${ADMIN}/spaces/${encodeURIComponent(spaceId)}/members/${encodeURIComponent(userId)}`,
        role === undefined ? {} : { role },
      ),
    async groups() {
      const list = await http.get<GroupList>(`${ADMIN}/groups`);
      return list.groups;
    },
    createGroup: (name, description) =>
      http.send<GroupView>("POST", `${ADMIN}/groups`, { name, description }),
    updateGroup: (id, change) =>
      http.send<GroupView>("PATCH", `${ADMIN}/groups/${encodeURIComponent(id)}`, change),
    async deleteGroup(id) {
      await http.send("DELETE", `${ADMIN}/groups/${encodeURIComponent(id)}`);
    },
    groupMembers: (id) =>
      http.get<GroupMembersView>(`${ADMIN}/groups/${encodeURIComponent(id)}/members`),
    signIn: {
      settings: () => http.get<SignInView>(`${ADMIN}/sign-in`),
      update: (change) => http.send<SignInView>("PATCH", `${ADMIN}/sign-in`, change),
      resetFactors: (userId) =>
        http.send<UserView>(
          "POST",
          `${ADMIN}/users/${encodeURIComponent(userId)}/second-factors/reset`,
        ),
      async signOutEverywhere(userId) {
        const done = await http.send<{ sessions: number }>(
          "POST",
          `${ADMIN}/users/${encodeURIComponent(userId)}/sign-out`,
        );
        return done.sessions;
      },
      addProvider: (input) => http.send<ProviderView>("POST", `${ADMIN}/sign-in/providers`, input),
      updateProvider: (id, input) =>
        http.send<ProviderView>("PUT", `${ADMIN}/sign-in/providers/${encodeURIComponent(id)}`, {
          provider: input,
        }),
      removeProvider: (id) =>
        http.send<SignInView>("DELETE", `${ADMIN}/sign-in/providers/${encodeURIComponent(id)}`),
      rotateScim: () => http.send<ScimTokenCreated>("POST", `${ADMIN}/sign-in/scim/token`),
      revokeScim: () => http.send<SignInView>("DELETE", `${ADMIN}/sign-in/scim/token`),
      samlMetadata: (url) =>
        http.send<SamlMetadataView>("POST", `${ADMIN}/sign-in/saml-metadata`, { url }),
    },
    tokens: {
      list: () => http.get<AdminTokenView[]>(`${ADMIN}/tokens`),
      async revoke(id) {
        await http.send("DELETE", `${ADMIN}/tokens/${encodeURIComponent(id)}`);
      },
      policy: () => http.get<TokenPolicy>("/api/v1/me/token-policy"),
      setPolicy: (maxDays) => http.send<TokenPolicy>("PUT", `${ADMIN}/token-policy`, { maxDays }),
    },
    changeGroupMembers: (id, change) =>
      http.send<GroupMembersView>(
        "POST",
        `${ADMIN}/groups/${encodeURIComponent(id)}/members`,
        change,
      ),
  };
}

/** A text field of a submitted form, trimmed; empty when missing. */
export function formText(data: FormData, name: string): string {
  const value = data.get(name);
  return typeof value === "string" ? value.trim() : "";
}

/** Whether a save lost a race with another admin's save. */
export function isConflict(error: unknown): boolean {
  return error instanceof ApiError && (error.code === "conflict" || error.status === 409);
}

export {
  type SignInView,
  type ProviderInput,
  type ProviderView,
  type AdminTokenView,
  type TokenPolicy,
  type GroupMembersView,
  type GroupView,
  type AdminSpaceView,
  type SettingsView,
  type SpaceRoleView,
  type UserView,
  type AdminSettings,
  type AdminState,
  type AuditRecord,
  type BackupFile,
  type ChainReport,
  type DrillReport,
  type SearchStatusView,
  type UsageCounts,
  type Versioned,
} from "../../api/api.gen";
