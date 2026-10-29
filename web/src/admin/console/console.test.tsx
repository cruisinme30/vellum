// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { ApiError, type Http } from "../../api/http";
import { I18nProvider } from "../../i18n/i18n";

import {
  createHttpAdminConsoleApi,
  type AdminConsoleApi,
  type AuditRecord,
  type UsageCounts,
  type Versioned,
} from "./api";
import { AuditPage } from "./audit-page";
import { BackupsPage } from "./backups-page";
import { AdminConsoleProvider } from "./context";
import { settingsFromForm, SettingsPage } from "./settings-page";
import { UsagePage } from "./usage-page";

function record(seq: number, overrides: Partial<AuditRecord["event"]> = {}): AuditRecord {
  return {
    seq,
    at: "2026-10-29T09:00:00Z",
    prev_hash: "p",
    hash: "h",
    event: {
      actor: { kind: "user", user_id: "u-priya" },
      action: "admin.settings_update",
      target: "settings",
      outcome: "success",
      request_id: `req-${String(seq)}`,
      details: {},
      ...overrides,
    },
  };
}

const SETTINGS: Versioned = {
  version: 3,
  settings: {
    organisation: "Northstar",
    telemetry: { enabled: false, install_id: "abc", interval_hours: 24 },
    observability: { slow_request_ms: 500, budgets: { read_ms: 200 } },
    backups: { enabled: true, interval_hours: 24, keep: 7, destination: "backups" },
    security: { fresh_auth_minutes: 15, session_hours: 12 },
  },
};

const USAGE: UsageCounts = {
  sampled_at_ms: Date.UTC(2026, 9, 29, 20, 30),
  users: 12,
  enabled_users: 11,
  admins: 2,
  active_users_1d: 4,
  active_users_7d: 9,
  active_users_30d: 11,
  sessions: 7,
  api_tokens: 3,
  spaces: 14,
  folders: 30,
  pages: 1250,
  books: 2,
  collections: 5,
  trashed: 6,
  versions: 400,
  file_bytes: 3 * 1024 * 1024,
  version_bytes: 1024 * 1024,
  events_1d: 57,
  events_7d: 310,
};

function fakeApi(overrides: Partial<AdminConsoleApi> = {}) {
  const audit = vi.fn<AdminConsoleApi["audit"]>(() =>
    Promise.resolve([record(2), record(1, { outcome: "denied" })]),
  );
  const settings = vi.fn<AdminConsoleApi["settings"]>(() => Promise.resolve(SETTINGS));
  const saveSettings = vi.fn<AdminConsoleApi["saveSettings"]>((version, next) =>
    Promise.resolve({ version: version + 1, settings: next }),
  );
  const state = vi.fn<AdminConsoleApi["state"]>(() =>
    Promise.resolve({
      last_backup: {
        name: "vellum-1.tar.age",
        created_at: "2026-10-29T08:00:00Z",
        files: 40,
        bytes: 9000,
        size: 4096,
        duration_ms: 50,
        skipped_links: 0,
      },
    }),
  );
  const api: AdminConsoleApi = {
    audit,
    verifyAudit: vi.fn(() => Promise.resolve({ records: 2, head: "h" })),
    exportAudit: vi.fn(() => Promise.resolve("{}\n")),
    settings,
    saveSettings,
    state,
    backups: vi.fn(() => Promise.resolve([{ name: "vellum-1.tar.age", size: 4096 }])),
    backUpNow: vi.fn(() =>
      Promise.resolve({
        name: "vellum-2.tar.age",
        created_at: "2026-10-29T10:00:00Z",
        files: 41,
        bytes: 9100,
        size: 4200,
        duration_ms: 60,
        skipped_links: 0,
      }),
    ),
    runDrill: vi.fn(() =>
      Promise.resolve({
        backup: "vellum-2.tar.age",
        ran_at: "2026-10-29T10:01:00Z",
        passed: true,
        files: 41,
        bytes: 9100,
        duration_ms: 80,
        within_rto: true,
      }),
    ),
    usage: vi.fn(() => Promise.resolve(USAGE)),
    users: vi.fn(() => Promise.resolve([])),
    createUser: vi.fn(() => Promise.reject(new Error("not in this test"))),
    updateUser: vi.fn(() => Promise.reject(new Error("not in this test"))),
    spaces: vi.fn(() => Promise.resolve([])),
    createSpace: vi.fn(() => Promise.reject(new Error("not in this test"))),
    updateSpace: vi.fn(() => Promise.reject(new Error("not in this test"))),
    setMember: vi.fn(() => Promise.reject(new Error("not in this test"))),
    groups: vi.fn(() => Promise.resolve([])),
    createGroup: vi.fn(() => Promise.reject(new Error("not in this test"))),
    updateGroup: vi.fn(() => Promise.reject(new Error("not in this test"))),
    deleteGroup: vi.fn(() => Promise.reject(new Error("not in this test"))),
    groupMembers: vi.fn(() => Promise.reject(new Error("not in this test"))),
    changeGroupMembers: vi.fn(() => Promise.reject(new Error("not in this test"))),
    ...overrides,
  };
  return { api, audit, settings, saveSettings, state };
}

function renderWith(api: AdminConsoleApi | undefined, page: ReactNode) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <AdminConsoleProvider api={api}>{page}</AdminConsoleProvider>
    </I18nProvider>,
  );
  return user;
}

describe("admin console API", () => {
  it("calls the admin routes", async () => {
    const answers: Record<string, unknown> = {
      "/api/v1/admin/audit": { records: [] },
      "/api/v1/admin/audit/export": { jsonl: "x" },
    };
    const get = vi.fn((path: string) => Promise.resolve(answers[path] ?? { backups: [] }));
    const send = vi.fn(() => Promise.resolve({}));
    const api = createHttpAdminConsoleApi({ get, send, forgetSession: vi.fn() } as unknown as Http);
    await api.audit({ action: "admin.backup", before: 10, limit: 50 });
    expect(get).toHaveBeenCalledWith("/api/v1/admin/audit", {
      action: "admin.backup",
      before: 10,
      limit: 50,
    });
    expect(await api.exportAudit()).toBe("x");
    await api.saveSettings(3, { organisation: "N" });
    expect(send).toHaveBeenCalledWith("PUT", "/api/v1/admin/settings", {
      expected_version: 3,
      settings: { organisation: "N" },
    });
    await api.runDrill();
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/admin/backups/drill");
    await api.usage(false);
    expect(get).toHaveBeenLastCalledWith("/api/v1/admin/usage", {});
    await api.usage(true);
    expect(get).toHaveBeenLastCalledWith("/api/v1/admin/usage", { refresh: "true" });
  });
});

describe("usage page", () => {
  it("shows people, activity, content and storage, and counts again on demand", async () => {
    const usage = vi.fn<AdminConsoleApi["usage"]>(() => Promise.resolve(USAGE));
    const { api } = fakeApi({ usage });
    const user = renderWith(api, <UsagePage />);
    const people = await screen.findByRole("region", { name: "People" });
    expect(within(people).getByText("Accounts").nextSibling?.textContent).toBe("12");
    const content = screen.getByRole("region", { name: "Content" });
    expect(within(content).getByText("Pages").nextSibling?.textContent).toBe("1,250");
    const storage = screen.getByRole("region", { name: "Storage" });
    expect(within(storage).getByText("Total stored").nextSibling?.textContent).toBe("4.0 MB");
    const activity = screen.getByRole("region", { name: "Activity" });
    expect(within(activity).getByText("Changes this week").nextSibling?.textContent).toBe("310");
    expect(screen.getByRole("status").textContent).toMatch(/^Counted /);
    expect(usage).toHaveBeenLastCalledWith(false);
    await user.click(screen.getByRole("button", { name: "Count now" }));
    await waitFor(() => {
      expect(usage).toHaveBeenLastCalledWith(true);
    });
  });

  it("says when nothing was counted yet, and reports failures", async () => {
    const { api } = fakeApi({
      usage: vi.fn(() => Promise.resolve({ ...USAGE, sampled_at_ms: 0 })),
    });
    renderWith(api, <UsagePage />);
    expect(await screen.findByText("Nothing counted yet.")).toBeDefined();
  });

  it("reports a failed count", async () => {
    const { api } = fakeApi({
      usage: vi.fn(() =>
        Promise.reject(new ApiError("unavailable", "index down", { status: 503 })),
      ),
    });
    renderWith(api, <UsagePage />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/index down/);
  });
});

describe("audit page", () => {
  it("lists records, verifies the chain and filters", async () => {
    const { api, audit } = fakeApi();
    const user = renderWith(api, <AuditPage />);
    const table = await screen.findByRole("table", { name: /Audit records/ });
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(within(table).getByText("Denied")).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Verify the log" }));
    expect(await screen.findByText(/The log is intact: 2 records checked/)).toBeDefined();

    await user.type(screen.getByRole("textbox", { name: "Action" }), "admin.backup");
    await user.click(screen.getByRole("button", { name: "Filter" }));
    await waitFor(() => {
      expect(audit).toHaveBeenLastCalledWith({ action: "admin.backup", limit: 50 });
    });
  });

  it("explains itself in demo mode", () => {
    renderWith(undefined, <AuditPage />);
    expect(screen.getByText(/needs a Vellum server/)).toBeDefined();
  });
});

describe("settings page", () => {
  it("saves the form against the version read, keeping fields it does not show", async () => {
    const { api, saveSettings } = fakeApi();
    const user = renderWith(api, <SettingsPage />);
    const keep = await screen.findByRole("spinbutton", { name: "Backups to keep" });
    await user.clear(keep);
    await user.type(keep, "14");
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Theme for exports (PDF, HTML, Word)" }),
      "Sepia",
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Settings saved.")).toBeDefined();
    const [version, settings] = saveSettings.mock.calls[0] ?? [];
    expect(version).toBe(3);
    expect(settings?.backups?.keep).toBe(14);
    expect(settings?.observability?.budgets).toEqual({ read_ms: 200 });
    expect(settings?.telemetry?.install_id).toBe("abc");
    expect(settings?.appearance).toEqual({
      seed: "#4f5b92",
      export_theme: "sepia",
      site_theme: "auto",
    });
  });

  it("says when another admin saved first", async () => {
    const { api, settings } = fakeApi({
      saveSettings: () =>
        Promise.reject(new ApiError("conflict", "stale version", { status: 409 })),
    });
    const user = renderWith(api, <SettingsPage />);
    await user.click(await screen.findByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Another admin saved");
    await user.click(screen.getByRole("button", { name: "Load the latest settings" }));
    await waitFor(() => {
      expect(settings).toHaveBeenCalledTimes(2);
    });
  });

  it("reads an empty report address as none", () => {
    const data = new FormData();
    data.set("telemetry.endpoint", "  ");
    data.set("telemetry.interval_hours", "6");
    const settings = settingsFromForm(SETTINGS.settings, data);
    expect(settings.telemetry?.endpoint).toBeUndefined();
    expect(settings.telemetry?.interval_hours).toBe(6);
    expect(settings.telemetry?.enabled).toBe(false);
  });
});

describe("backups page", () => {
  it("shows the last backup and runs a backup and a drill", async () => {
    const { api, state } = fakeApi();
    const user = renderWith(api, <BackupsPage />);
    expect(await screen.findByText(/40 files, 4.0 KB/)).toBeDefined();
    expect(screen.getByRole("table", { name: /Backups kept/ })).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Back up now" }));
    expect(await screen.findByText(/Backup vellum-2.tar.age written/)).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Run a restore drill" }));
    expect(await screen.findByText(/The restore drill passed: 41 files/)).toBeDefined();
    expect(state).toHaveBeenCalledTimes(3);
  });
});
