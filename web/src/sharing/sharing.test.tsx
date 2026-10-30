// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { I18nProvider } from "../i18n/i18n";
import { createListDirectory } from "../people/directory";

import {
  createHttpSharingApi,
  type GrantView,
  type LinkView,
  type SharingApi,
  type SharingView,
} from "./api";
import { endOfDay } from "./dates";
import { SharePanel } from "./share-panel";

const PEOPLE = createListDirectory([
  { kind: "user", principal: "user:01PRIYA", name: "Priya Raman", detail: "priya@northstar.dev" },
  { kind: "group", principal: "group:01SRE", name: "SRE", detail: "12 members" },
]);

/** A link password typed in the test. */
const PHRASE = ["northstar", "2026"].join("-");

const HANDLES: Record<string, string> = { "user:01SAM": "sam", "user:01PRIYA": "priya" };

/** A grant's optional fields, left out when unset. */
function pick(handle: string | undefined, expiresAt: string | undefined) {
  return {
    ...(handle === undefined ? {} : { handle }),
    ...(expiresAt === undefined ? {} : { expires_at: expiresAt }),
  };
}

/** Sharing in memory, as the server answers it. */
function memorySharing(kind: "page" | "folder" = "page", linksAllowed = true) {
  let grants: GrantView[] = [{ principal: "user:01SAM", handle: "sam", role: "owner" }];
  let links: LinkView[] = [];
  let next = 0;
  const view = (): SharingView =>
    ({
      item: { id: "pg1", kind, title: "Roadmap" },
      grants: [...grants],
      links: [...links],
      links_allowed: linksAllowed,
    }) as unknown as SharingView;
  const api: SharingApi = {
    state: () => Promise.resolve(view()),
    share(_id, principal, role, expiresAt) {
      if (principal === "user:01FAIL") return Promise.reject(new Error("Not allowed."));
      grants = [
        ...grants.filter((g) => g.principal !== principal),
        { principal, role, ...pick(HANDLES[principal], expiresAt) },
      ];
      return Promise.resolve(view());
    },
    unshare(_id, principal) {
      grants = grants.filter((g) => g.principal !== principal);
      return Promise.resolve(view());
    },
    createLink(_id, link) {
      next += 1;
      const made: LinkView = {
        id: `link${String(next)}`,
        has_password: link.password !== undefined,
        ...(link.expiresAt === undefined ? {} : { expires_at: link.expiresAt }),
        created_by: "01SAM",
        created_at: "2026-10-29T17:00:00Z",
        live: true,
      };
      links = [made, ...links];
      const token = `tok${String(next)}`;
      return Promise.resolve({
        link: made,
        token,
        url: `https://pages.northstar.dev/s/${token}`,
      });
    },
    revokeLink(linkId) {
      links = links.map((l) => (l.id === linkId ? { ...l, live: false } : l));
      return Promise.resolve();
    },
  };
  return api;
}

function renderPanel(api: SharingApi, onClose = vi.fn()) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <SharePanel api={api} itemId="pg1" directory={PEOPLE} onClose={onClose} />
    </I18nProvider>,
  );
  return { user, onClose };
}

describe("share dialog", () => {
  it("adds a person with a role, changes it and removes them", async () => {
    const api = memorySharing();
    const share = vi.spyOn(api, "share");
    const { user } = renderPanel(api);
    expect(await screen.findByRole("heading", { name: "Share Roadmap" })).toBeDefined();
    const access = screen.getByRole("region", { name: "People with access" });
    expect(within(access).getByText("@sam")).toBeDefined();
    expect(within(access).getByText("Owner")).toBeDefined();

    await user.type(screen.getByRole("combobox", { name: "People or groups" }), "pri");
    await user.click(await screen.findByRole("option", { name: /Priya Raman/u }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Role" }), "editor");
    await user.click(screen.getByRole("button", { name: "Share" }));
    expect(share).toHaveBeenCalledWith("pg1", "user:01PRIYA", "editor", undefined);
    expect(await within(access).findByText("@priya")).toBeDefined();

    await user.selectOptions(
      within(access).getByRole("combobox", { name: "Role for @priya" }),
      "viewer",
    );
    expect(share).toHaveBeenLastCalledWith("pg1", "user:01PRIYA", "viewer");

    await user.click(within(access).getByRole("button", { name: "Remove @priya" }));
    expect(within(access).queryByText("@priya")).toBeNull();
  });

  it("shows why a change failed", async () => {
    const api = memorySharing();
    const { user } = renderPanel(api);
    await user.type(
      await screen.findByRole("combobox", { name: "People or groups" }),
      "user:01FAIL{Enter}",
    );
    await user.click(screen.getByRole("button", { name: "Share" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Not allowed.");
  });

  it("makes a password link, shows it once, and revokes it", async () => {
    const api = memorySharing();
    const create = vi.spyOn(api, "createLink");
    const { user } = renderPanel(api);
    const links = await screen.findByRole("region", { name: "Share links" });
    await user.type(within(links).getByLabelText("Password (optional)"), PHRASE);
    await user.click(within(links).getByRole("button", { name: "Create link" }));
    expect(create).toHaveBeenCalledWith("pg1", { password: PHRASE });
    const made = await within(links).findByLabelText("New link");
    expect((made as HTMLInputElement).value).toBe("https://pages.northstar.dev/s/tok1");
    expect(within(links).getByText(/only time/u)).toBeDefined();
    expect(within(links).getByText(/Active · Password/u)).toBeDefined();

    await user.click(within(links).getByRole("button", { name: "Revoke" }));
    expect(await within(links).findByText(/Revoked/u)).toBeDefined();
    expect(within(links).queryByLabelText("New link")).toBeNull();
  });

  it("explains when links are off, and has none for folders", async () => {
    renderPanel(memorySharing("page", false));
    expect(await screen.findByText(/public address/u)).toBeDefined();
  });

  it("offers no links on a folder, and closes on Escape", async () => {
    const { user, onClose } = renderPanel(memorySharing("folder"));
    await screen.findByRole("heading", { name: "Share Roadmap" });
    expect(screen.queryByRole("region", { name: "Share links" })).toBeNull();
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});

describe("sharing api", () => {
  it("calls the drive's sharing routes", async () => {
    const get = vi.fn().mockResolvedValue({});
    const send = vi.fn().mockResolvedValue({});
    const api = createHttpSharingApi({ get, send } as unknown as Http);
    await api.state("pg 1");
    expect(get).toHaveBeenCalledWith("/api/v1/drive/items/pg%201/sharing");
    await api.share("pg1", "group:01SRE", "commenter");
    expect(send).toHaveBeenCalledWith("PUT", "/api/v1/drive/items/pg1/grants", {
      principal: "group:01SRE",
      role: "commenter",
      expires_at: undefined,
    });
    await api.unshare("pg1", "user:01PRIYA");
    expect(send).toHaveBeenCalledWith("DELETE", "/api/v1/drive/items/pg1/grants/user%3A01PRIYA");
    await api.createLink("pg1", { expiresAt: "2026-11-01T06:59:59.000Z" });
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/drive/items/pg1/links", {
      expires_at: "2026-11-01T06:59:59.000Z",
      password: undefined,
    });
    await api.revokeLink("link1");
    expect(send).toHaveBeenLastCalledWith("DELETE", "/api/v1/drive/links/link1");
  });

  it("ends a picked day at its last second, and rejects junk", () => {
    const end = endOfDay("2026-11-01");
    expect(end && new Date(end).getDate()).toBe(1);
    expect(end && new Date(end).getHours()).toBe(23);
    expect(endOfDay("soon")).toBeUndefined();
  });
});
