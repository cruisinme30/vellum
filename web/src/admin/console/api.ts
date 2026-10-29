// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The admin console's audit, settings and backup operations (`/api/v1/admin`,
 * issue #254), over the app's HTTP client so writes carry the CSRF token.
 * Types come from the server's generated client.
 */

import type {
  AdminSettings,
  AdminState,
  AuditList,
  AuditQuery,
  AuditRecord,
  BackupFile,
  BackupInfo,
  ChainReport,
  DrillReport,
  UsageCounts,
  Versioned,
} from "../../api/api.gen";
import { ApiError, type Http, type Query } from "../../api/http";

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
  type AdminSettings,
  type AdminState,
  type AuditRecord,
  type BackupFile,
  type ChainReport,
  type DrillReport,
  type UsageCounts,
  type Versioned,
} from "../../api/api.gen";
