// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../../api/http";
import { I18nProvider } from "../../i18n/i18n";

import {
  createHttpAdminConsoleApi,
  type AdminConsoleApi,
  type AdminSpaceView,
  type UserView,
} from "./api";
import { AdminConsoleProvider } from "./context";
import { settingsFromSpaceForm } from "./space-panel";
import { slugOf, SpacesPage } from "./spaces-page";
import { newUserFromForm, UsersPage } from "./users-page";

const PRIYA: UserView = {
  id: "01JBQ8Y4ZK6V3W2N5R7T9X1C01",
  email: "priya@northstar.dev",
  name: "Priya Shah",
  handle: "priya",
  admin: true,
  disabled: false,
  hasPassword: true,
  createdAt: "2026-10-01T09:00:00Z",
};

const MARCO: UserView = {
  id: "01JBQ8Y4ZK6V3W2N5R7T9X1C02",
  email: "marco@northstar.dev",
  name: "Marco Ruiz",
  handle: "marco",
  admin: false,
  disabled: false,
  hasPassword: false,
  createdAt: "2026-10-02T09:00:00Z",
};

const RETENTION = { trash_days: 30, keep_all_days: 7, daily_days: 30 };
const SETTINGS = { ...RETENTION, quota_bytes: 10 * 1024 * 1024 };

const ENG: AdminSpaceView = {
  space: {
    id: "01JBQ8Y4ZK6V3W2N5R7T9X1C10",
    kind: "shared_drive",
    slug: "engineering",
    name: "Engineering",
    settings: SETTINGS,
  },
  used_bytes: 2 * 1024 * 1024,
  members: [{ user_id: PRIYA.id, role: "owner" }],
};

const MINE: AdminSpaceView = {
  space: {
    id: "01JBQ8Y4ZK6V3W2N5R7T9X1C11",
    kind: "user_drive",
    slug: "marco",
    name: "My Drive",
    owner: MARCO.id,
    settings: RETENTION,
  },
  used_bytes: 0,
  members: [{ user_id: MARCO.id, role: "owner" }],
};

const unused = () => Promise.reject(new Error("not in this test"));

function fakeApi(overrides: Partial<AdminConsoleApi> = {}) {
  const updateUser = vi.fn<AdminConsoleApi["updateUser"]>((id, change) =>
    Promise.resolve({ ...(id === PRIYA.id ? PRIYA : MARCO), ...change }),
  );
  const createUser = vi.fn<AdminConsoleApi["createUser"]>((input) =>
    Promise.resolve({ ...MARCO, id: "new", name: input.name, email: input.email }),
  );
  const createSpace = vi.fn<AdminConsoleApi["createSpace"]>(() => Promise.resolve(ENG));
  const setMember = vi.fn<AdminConsoleApi["setMember"]>(() => Promise.resolve(ENG));
  const api: AdminConsoleApi = {
    audit: unused,
    verifyAudit: unused,
    exportAudit: unused,
    settings: unused,
    saveSettings: unused,
    state: unused,
    backups: unused,
    backUpNow: unused,
    runDrill: unused,
    usage: unused,
    users: () => Promise.resolve([PRIYA, MARCO]),
    createUser,
    updateUser,
    spaces: () => Promise.resolve([ENG, MINE]),
    createSpace,
    updateSpace: () => Promise.resolve(ENG),
    setMember,
    groups: unused,
    createGroup: unused,
    updateGroup: unused,
    deleteGroup: unused,
    groupMembers: unused,
    changeGroupMembers: unused,
    ...overrides,
  };
  return { api, updateUser, createUser, createSpace, setMember };
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

describe("admin users and spaces API", () => {
  it("calls the admin routes", async () => {
    const get = vi.fn(() => Promise.resolve({ spaces: [ENG] }));
    const send = vi.fn(() => Promise.resolve(ENG));
    const api = createHttpAdminConsoleApi({ get, send, forgetSession: vi.fn() } as unknown as Http);
    expect(await api.spaces()).toEqual([ENG]);
    expect(get).toHaveBeenCalledWith("/api/v1/admin/spaces");
    await api.updateUser(MARCO.id, { disabled: true });
    expect(send).toHaveBeenLastCalledWith("PATCH", `/api/v1/admin/users/${MARCO.id}`, {
      disabled: true,
    });
    await api.setMember(ENG.space.id, MARCO.id, undefined);
    expect(send).toHaveBeenLastCalledWith(
      "PUT",
      `/api/v1/admin/spaces/${ENG.space.id}/members/${MARCO.id}`,
      {},
    );
  });
});

describe("users page", () => {
  it("lists accounts and blocks only after a confirmation", async () => {
    const { api, updateUser } = fakeApi();
    const user = renderWith(api, <UsersPage />);
    const table = await screen.findByRole("table", { name: /2 accounts/ });
    const marco = within(table).getByRole("row", { name: /Marco Ruiz/ });
    expect(within(marco).getByText("Single sign-on")).toBeDefined();

    await user.click(within(marco).getByRole("button", { name: "Block Marco Ruiz" }));
    expect(updateUser).not.toHaveBeenCalled();
    await user.click(within(marco).getByRole("button", { name: "Block" }));
    await waitFor(() => {
      expect(updateUser).toHaveBeenCalledWith(MARCO.id, { disabled: true });
    });
    expect(await screen.findByText("Marco Ruiz updated.")).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Remove Priya Shah's admin rights" }));
    expect(updateUser).toHaveBeenLastCalledWith(PRIYA.id, { admin: false });
  });

  it("adds a person, with no password for single sign-on", async () => {
    const { api, createUser } = fakeApi();
    const user = renderWith(api, <UsersPage />);
    await user.type(await screen.findByRole("textbox", { name: "Name" }), "Ana Lima");
    await user.type(screen.getByRole("textbox", { name: "Email" }), "ana@northstar.dev");
    await user.click(screen.getByRole("button", { name: "Add person" }));
    await waitFor(() => {
      expect(createUser).toHaveBeenCalledWith({
        name: "Ana Lima",
        email: "ana@northstar.dev",
        admin: false,
      });
    });
    expect(await screen.findByText("Ana Lima can now sign in.")).toBeDefined();
  });

  it("shows the server's refusal", async () => {
    const { api } = fakeApi({
      updateUser: () => Promise.reject(new Error("x")),
    });
    const user = renderWith(api, <UsersPage />);
    await user.click(await screen.findByRole("button", { name: "Make Marco Ruiz an admin" }));
    expect(await screen.findByRole("alert")).toBeDefined();
  });

  it("reads the password only when one is given", () => {
    const data = new FormData();
    data.set("name", " Ana ");
    data.set("email", "ana@northstar.dev");
    data.set("password", "");
    expect(newUserFromForm(data)).toEqual({
      name: "Ana",
      email: "ana@northstar.dev",
      admin: false,
    });
    data.set("password", "correct horse battery");
    data.set("admin", "on");
    expect(newUserFromForm(data)).toMatchObject({ password: "correct horse battery", admin: true });
  });

  it("explains itself in demo mode", () => {
    renderWith(undefined, <UsersPage />);
    expect(screen.getByText(/needs a Vellum server/)).toBeDefined();
  });
});

describe("spaces page", () => {
  it("lists spaces and changes a member's role", async () => {
    const { api, setMember } = fakeApi();
    const user = renderWith(api, <SpacesPage />);
    const table = await screen.findByRole("table", { name: /2 spaces/ });
    const eng = within(table).getByRole("row", { name: /Engineering/ });
    expect(within(eng).getByText("2.0 MB of 10.0 MB")).toBeDefined();
    expect(within(eng).getByText("Priya Shah")).toBeDefined();

    await user.click(within(eng).getByRole("button", { name: "Manage Engineering" }));
    const panel = screen.getByRole("region", { name: /Settings and members of Engineering/ });
    await user.selectOptions(
      within(panel).getByRole("combobox", { name: "Role of Priya Shah" }),
      "manager",
    );
    await waitFor(() => {
      expect(setMember).toHaveBeenCalledWith(ENG.space.id, PRIYA.id, "manager");
    });

    await user.selectOptions(within(panel).getByRole("combobox", { name: "Person" }), MARCO.id);
    await user.click(within(panel).getByRole("button", { name: "Add member" }));
    await waitFor(() => {
      expect(setMember).toHaveBeenLastCalledWith(ENG.space.id, MARCO.id, "editor");
    });
  });

  it("keeps a My Drive's members to its owner", async () => {
    const { api } = fakeApi();
    const user = renderWith(api, <SpacesPage />);
    await user.click(await screen.findByRole("button", { name: "Manage My Drive" }));
    expect(screen.getByText(/A My Drive belongs to its owner/)).toBeDefined();
    expect(screen.queryByRole("button", { name: "Add member" })).toBeNull();
  });

  it("creates a shared drive with a folder name made from its name", async () => {
    const { api, createSpace } = fakeApi();
    const user = renderWith(api, <SpacesPage />);
    const form = await screen.findByRole("form", { name: "Create a shared drive" });
    await user.type(within(form).getByRole("textbox", { name: "Name" }), "Field Ops");
    expect(within(form).getByRole("textbox", { name: /Folder name/ })).toHaveProperty(
      "value",
      "field-ops",
    );
    await user.selectOptions(within(form).getByRole("combobox", { name: "Owner" }), MARCO.id);
    await user.click(within(form).getByRole("button", { name: "Create a shared drive" }));
    await waitFor(() => {
      expect(createSpace).toHaveBeenCalledWith({
        name: "Field Ops",
        slug: "field-ops",
        owner_id: MARCO.id,
      });
    });
  });

  it("reads the quota in MiB and keeps the version retention", () => {
    const data = new FormData();
    data.set("quota", "");
    data.set("trash_days", "14");
    expect(settingsFromSpaceForm(SETTINGS, data)).toEqual({ ...RETENTION, trash_days: 14 });
    data.set("quota", "1.5");
    expect(settingsFromSpaceForm(SETTINGS, data).quota_bytes).toBe(1_572_864);
  });

  it("makes folder names", () => {
    expect(slugOf("  Field Ops / North ")).toBe("field-ops-north");
    expect(slugOf("Équipe")).toBe("equipe");
  });
});
