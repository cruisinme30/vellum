// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../../api/http";
import { I18nProvider } from "../../i18n/i18n";

import {
  createHttpAdminConsoleApi,
  type AdminConsoleApi,
  type GroupMembersView,
  type GroupView,
  type UserView,
} from "./api";
import { AdminConsoleProvider } from "./context";
import { GroupsPage } from "./groups-page";

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
  ...PRIYA,
  id: "01JBQ8Y4ZK6V3W2N5R7T9X1C02",
  name: "Marco Ruiz",
  handle: "marco",
  email: "marco@northstar.dev",
  admin: false,
};

const SRE: GroupView = {
  id: "01JBQ8Y4ZK6V3W2N5R7T9X1C20",
  name: "SRE",
  description: "On call",
  created_at: "2026-10-29T09:00:00Z",
};

const unused = () => Promise.reject(new Error("not in this test"));

function fakeApi() {
  const members: GroupMembersView = { group: SRE, members: [PRIYA] };
  const changeGroupMembers = vi.fn<AdminConsoleApi["changeGroupMembers"]>(() =>
    Promise.resolve({ group: SRE, members: [PRIYA, MARCO] }),
  );
  const createGroup = vi.fn<AdminConsoleApi["createGroup"]>(
    (name, description) =>
      Promise.resolve({ ...SRE, id: "new", name, description }),
  );
  const deleteGroup = vi.fn<AdminConsoleApi["deleteGroup"]>(() =>
    Promise.resolve(),
  );
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
    createUser: unused,
    updateUser: unused,
    spaces: unused,
    createSpace: unused,
    updateSpace: unused,
    setMember: unused,
    groups: () => Promise.resolve([SRE]),
    createGroup,
    updateGroup: (_id, change) => Promise.resolve({ ...SRE, ...change }),
    deleteGroup,
    groupMembers: () => Promise.resolve(members),
    changeGroupMembers,
  };
  return { api, changeGroupMembers, createGroup, deleteGroup };
}

function renderPage(api: AdminConsoleApi) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <AdminConsoleProvider api={api}>
        <GroupsPage />
      </AdminConsoleProvider>
    </I18nProvider>,
  );
  return user;
}

describe("groups API", () => {
  it("calls the admin routes", async () => {
    const send = vi.fn(() => Promise.resolve({}));
    const api = createHttpAdminConsoleApi({
      get: vi.fn(),
      send,
      forgetSession: vi.fn(),
    } as unknown as Http);
    await api.changeGroupMembers(SRE.id, { remove: [MARCO.id] });
    expect(send).toHaveBeenLastCalledWith(
      "POST",
      `/api/v1/admin/groups/${SRE.id}/members`,
      {
        remove: [MARCO.id],
      },
    );
    await api.deleteGroup(SRE.id);
    expect(send).toHaveBeenLastCalledWith(
      "DELETE",
      `/api/v1/admin/groups/${SRE.id}`,
    );
  });
});

describe("groups page", () => {
  it("adds a member and deletes a group only after a confirmation", async () => {
    const { api, changeGroupMembers, deleteGroup } = fakeApi();
    const user = renderPage(api);
    await user.click(await screen.findByRole("button", { name: "Manage SRE" }));
    const panel = screen.getByRole("region", {
      name: /Name and members of SRE/,
    });
    expect(await within(panel).findByText("Priya Shah")).toBeDefined();

    await user.selectOptions(
      within(panel).getByRole("combobox", { name: "Add to group" }),
      MARCO.id,
    );
    await user.click(
      within(panel).getByRole("button", { name: "Add to group" }),
    );
    await waitFor(() => {
      expect(changeGroupMembers).toHaveBeenCalledWith(SRE.id, {
        add: [MARCO.id],
      });
    });
    expect(await within(panel).findByText("Marco Ruiz")).toBeDefined();

    await user.click(
      within(panel).getByRole("button", { name: "Delete group" }),
    );
    expect(deleteGroup).not.toHaveBeenCalled();
    await user.click(
      within(panel).getByRole("button", { name: "Delete group" }),
    );
    await waitFor(() => {
      expect(deleteGroup).toHaveBeenCalledWith(SRE.id);
    });
  });

  it("creates a group", async () => {
    const { api, createGroup } = fakeApi();
    const user = renderPage(api);
    const form = await screen.findByRole("form", { name: "Create a group" });
    await user.type(
      within(form).getByRole("textbox", { name: "Name" }),
      "Design",
    );
    await user.click(
      within(form).getByRole("button", { name: "Create a group" }),
    );
    await waitFor(() => {
      expect(createGroup).toHaveBeenCalledWith("Design", "");
    });
    expect(await screen.findByText("Design created.")).toBeDefined();
  });
});
