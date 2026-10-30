// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import {
  createHttpDriveApi,
  findQuery,
  type DriveApi,
  type NodeView,
  type SpaceView,
} from "./api";
import { MyDrivePage, SharedWithMePage, SpacePage } from "./drive-page";
import { actorText, formatBytes, usedShare } from "./format";
import { destinations } from "./place-dialog";
import { DriveSearchPage, filtersOf, parametersOf } from "./search-page";
import { DriveSettingsPage, draftOf, settingsOf } from "./settings-page";
import { purgeDate, TrashPage } from "./trash-page";

const SETTINGS = {
  trash_days: 30,
  keep_all_days: 7,
  daily_days: 30,
  quota_bytes: 1024 ** 3,
};
const MINE: SpaceView = {
  id: "s1",
  kind: "user_drive",
  slug: "ada",
  name: "My Drive",
  settings: SETTINGS,
};
const TEAM: SpaceView = {
  id: "s2",
  kind: "shared_drive",
  slug: "team",
  name: "Northstar Team",
  settings: { ...SETTINGS, weekly_weeks: 12 },
};

function node(
  id: string,
  title: string,
  fields: Partial<NodeView> = {},
): NodeView {
  return {
    id,
    space_id: "s1",
    kind: "page",
    title,
    path: `ada/${title}.md`,
    size: 2048,
    modified: "2026-10-29T18:00:00Z",
    ...fields,
  };
}

const PLAN = node("n1", "Launch plan");
const DOCS = node("f1", "Docs", { kind: "folder", path: "ada/Docs" });
const LINK = node("k1", "Plan shortcut", { kind: "shortcut", target_id: "n1" });

function fakeApi(overrides: Partial<DriveApi> = {}): DriveApi {
  const resolved = <T,>(value: T) => vi.fn(() => Promise.resolve(value));
  return {
    myDrive: resolved(MINE),
    sharedDrives: resolved([TEAM]),
    sharedWithMe: resolved([node("x1", "Shared spec", { space_id: "s9" })]),
    starred: resolved([PLAN]),
    recent: resolved([]),
    list: resolved([DOCS, PLAN, LINK]),
    get: vi.fn((id: string) => Promise.resolve(id === "f1" ? DOCS : PLAN)),
    containers: resolved({ items: [DOCS], truncated: false }),
    trashList: resolved([
      node("t1", "Old notes", { trashed_at: "2026-10-01T00:00:00Z" }),
    ]),
    activity: resolved([
      {
        at: "2026-10-29T18:00:00Z",
        actor: { type: "user", user: "u1" },
        item_id: "n1",
        kind: "created",
      },
      {
        at: "2026-10-29T19:00:00Z",
        actor: { type: "external" },
        item_id: "n1",
        kind: "renamed",
        from: "Plan",
        to: "Launch plan",
      },
    ]),
    backlinks: resolved([node("n2", "Roadmap")]),
    usage: resolved({ used_bytes: 0.95 * 1024 ** 3, quota_bytes: 1024 ** 3 }),
    find: resolved([PLAN]),
    target: resolved(PLAN),
    move: resolved(
      node("n1", "Launch plan", { path: "ada/Docs/Launch plan.md" }),
    ),
    copy: resolved(node("n3", "Launch plan", { path: "team/Launch plan.md" })),
    shortcut: resolved(node("k2", "Launch plan", { kind: "shortcut" })),
    createFolder: resolved(DOCS),
    rename: resolved(PLAN),
    star: vi.fn(() => Promise.resolve()),
    trash: resolved(PLAN),
    restore: resolved(PLAN),
    purge: vi.fn(() => Promise.resolve()),
    emptyTrash: resolved(1),
    updateSpace: resolved(MINE),
    pruneVersions: resolved(4),
    ...overrides,
  };
}

function Where() {
  const location = useLocation();
  return (
    <output data-testid="where">{location.pathname + location.search}</output>
  );
}

function renderAt(path: string, api: DriveApi | undefined) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ drive: api }}>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="drive" element={<MyDrivePage />} />
            <Route path="drive/shared-with-me" element={<SharedWithMePage />} />
            <Route path="drive/search" element={<DriveSearchPage />} />
            <Route path="drive/s/:spaceId" element={<SpacePage />} />
            <Route
              path="drive/s/:spaceId/f/:folderId"
              element={<SpacePage />}
            />
            <Route path="drive/s/:spaceId/trash" element={<TrashPage />} />
            <Route
              path="drive/s/:spaceId/settings"
              element={<DriveSettingsPage />}
            />
            <Route path="*" element={<p>elsewhere</p>} />
          </Routes>
          <Where />
        </MemoryRouter>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

const echo = (id: string) => id;

function fakeHttp() {
  const get = vi.fn<(path: string, query?: unknown) => Promise<unknown>>(() =>
    Promise.resolve({ items: [] }),
  );
  const send = vi.fn<
    (method: string, path: string, body?: unknown) => Promise<unknown>
  >(() => Promise.resolve({ removed: 2 }));
  return {
    http: { get, send, forgetSession: vi.fn() } as unknown as Http,
    get,
    send,
  };
}

describe("drive API", () => {
  it("calls the drive routes with the server's field names", async () => {
    const { http, get, send } = fakeHttp();
    const api = createHttpDriveApi(http);
    await api.list("s 1", "f/1");
    expect(get).toHaveBeenLastCalledWith("/api/v1/drive/spaces/s%201/items", {
      parent_id: "f/1",
    });
    await api.move("n1", { spaceId: "s2", parentId: "f1" });
    expect(send).toHaveBeenLastCalledWith(
      "POST",
      "/api/v1/drive/items/n1/move",
      {
        space_id: "s2",
        parent_id: "f1",
      },
    );
    await api.shortcut("n1", { spaceId: "s2" });
    expect(send).toHaveBeenLastCalledWith(
      "POST",
      "/api/v1/drive/spaces/s2/shortcuts",
      {
        parent_id: undefined,
        target_id: "n1",
      },
    );
    await api.purge("t1");
    expect(send).toHaveBeenLastCalledWith("DELETE", "/api/v1/drive/items/t1");
    expect(await api.emptyTrash("s1")).toBe(2);
    expect(send).toHaveBeenLastCalledWith(
      "DELETE",
      "/api/v1/drive/spaces/s1/trash",
    );
  });

  it("keeps only folders and pages as destinations", async () => {
    const { http, get } = fakeHttp();
    get.mockResolvedValueOnce({
      items: [DOCS, PLAN, node("b1", "Book", { kind: "book" })],
      truncated: true,
    });
    const found = await createHttpDriveApi(http).containers("s1");
    expect(found.items.map((item) => item.id)).toEqual(["f1", "n1"]);
    expect(found.truncated).toBe(true);
    expect(get).toHaveBeenLastCalledWith("/api/v1/drive/spaces/s1/tree", {
      limit: 10_000,
    });
  });

  it("builds the find query from filters and leaves empty ones out", () => {
    const query = findQuery({
      title: "  plan ",
      kinds: [],
      after: "2026-10-01",
      before: "bad",
    });
    expect(query["title"]).toBe("plan");
    expect(query["kinds"]).toBeUndefined();
    expect(query["modified_before"]).toBeUndefined();
    expect(new Date(String(query["modified_after"])).getDate()).toBe(1);
    const until = findQuery({ before: "2026-10-01" })["modified_before"];
    expect(new Date(String(until)).getDate()).toBe(2);
  });
});

describe("drive helpers", () => {
  it("formats sizes, shares and actors", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(usedShare(5, 0)).toBeUndefined();
    expect(usedShare(20, 10)).toBe(1);
    expect(actorText(echo, { type: "user", user: "u1" }, "u1")).toBe(
      "drive-actor-you",
    );
    expect(actorText(echo, { type: "system" }, "u1")).toBe(
      "drive-actor-system",
    );
  });

  it("never offers moving an item into itself or under itself", () => {
    const inner = node("f2", "Inner", { kind: "folder", parent_id: "f1" });
    const other = node("f3", "Other", { kind: "folder" });
    const all = [DOCS, inner, other];
    expect(destinations(all, DOCS, "move").map((d) => d.node.id)).toEqual([
      "f3",
    ]);
    expect(
      destinations(all, DOCS, "copy").map((d) => [d.node.id, d.depth]),
    ).toEqual([
      ["f1", 0],
      ["f2", 1],
      ["f3", 0],
    ]);
  });

  it("round-trips space settings and rejects bad numbers", () => {
    const draft = draftOf(TEAM);
    expect(draft.quotaGb).toBe("1");
    expect(settingsOf(draft)).toEqual({
      quota_bytes: 1024 ** 3,
      trash_days: 30,
      keep_all_days: 7,
      daily_days: 30,
      weekly_weeks: 12,
    });
    expect(settingsOf({ ...draft, quotaGb: "", weeklyWeeks: "" })).toEqual({
      trash_days: 30,
      keep_all_days: 7,
      daily_days: 30,
    });
    expect(settingsOf({ ...draft, trashDays: "0" })).toEqual({
      invalid: "trashDays",
    });
    expect(settingsOf({ ...draft, quotaGb: "-1" })).toEqual({
      invalid: "quotaGb",
    });
    expect(settingsOf({ ...draft, dailyDays: "1.5" })).toEqual({
      invalid: "dailyDays",
    });
  });

  it("keeps search filters in the URL", () => {
    const filters = {
      title: "plan",
      kinds: ["page", "book"] as const,
      starred: true,
    };
    const back = filtersOf(
      parametersOf({ ...filters, kinds: [...filters.kinds] }),
    );
    expect(back).toMatchObject({
      title: "plan",
      kinds: ["page", "book"],
      starred: true,
    });
    expect(filtersOf(new URLSearchParams("kind=nonsense")).kinds).toEqual([]);
  });

  it("dates the purge from the trash days", () => {
    expect(purgeDate("2026-10-01T00:00:00Z", 30)).toBe(
      "2026-10-31T00:00:00.000Z",
    );
    expect(purgeDate("never", 30)).toBeUndefined();
  });
});

describe("Drive screens", () => {
  it("says the Drive needs a server in demo mode", () => {
    renderAt("/drive", undefined as DriveApi | undefined);
    expect(screen.getByText(/needs a Vellum server/)).toBeDefined();
  });

  it("shows My Drive with its places, items, usage and a nearly-full warning", async () => {
    renderAt("/drive", fakeApi());
    const plan = await screen.findByRole("link", { name: "Launch plan" });
    expect(plan.getAttribute("href")).toBe("/p/n1");
    const places = screen.getByRole("navigation", { name: "Drive" });
    expect(
      within(places)
        .getByRole("link", { name: "My Drive" })
        .getAttribute("aria-current"),
    ).toBe("page");
    expect(
      screen.getByRole("link", { name: "Docs" }).getAttribute("href"),
    ).toBe("/drive/s/s1/f/f1");
    expect(await screen.findByText("Nearly full")).toBeDefined();
    expect(
      screen
        .getByRole("button", { name: "Unstar “Launch plan”" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("moves an item to a chosen folder and reloads", async () => {
    const api = fakeApi();
    const user = renderAt("/drive", api);
    await user.click(
      await screen.findByRole("button", { name: "Move “Launch plan”" }),
    );
    const dialog = screen.getByRole("dialog", { name: "Move “Launch plan”" });
    await user.selectOptions(within(dialog).getByLabelText("Drive"), "s2");
    await within(dialog).findByRole("option", { name: /Docs/ });
    await user.selectOptions(within(dialog).getByLabelText("Folder"), "f1");
    await user.click(within(dialog).getByRole("button", { name: "Move" }));
    expect(api.move).toHaveBeenCalledWith("n1", {
      spaceId: "s2",
      parentId: "f1",
    });
    expect(
      await screen.findByText(
        "Moved “Launch plan” to ada/Docs/Launch plan.md.",
      ),
    ).toBeDefined();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.list).toHaveBeenCalledTimes(2);
  });

  it("closes the move dialog with Escape and shows a refusal", async () => {
    const api = fakeApi({
      copy: vi.fn(() => Promise.reject(new Error("Quota exceeded"))),
    });
    const user = renderAt("/drive", api);
    await user.click(
      await screen.findByRole("button", { name: "Copy “Launch plan”" }),
    );
    await user.click(screen.getByRole("button", { name: "Copy" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Quota exceeded");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("stars, trashes and shows details with activity and backlinks", async () => {
    const api = fakeApi();
    const user = renderAt("/drive", api);
    await user.click(
      await screen.findByRole("button", { name: "Star “Docs”" }),
    );
    expect(api.star).toHaveBeenCalledWith("f1", true);
    await user.click(
      screen.getByRole("button", { name: "Details of “Launch plan”" }),
    );
    const details = screen.getByRole("complementary", { name: "Launch plan" });
    expect(
      await within(details).findByText(
        /renamed it from “Plan” to “Launch plan”/,
      ),
    ).toBeDefined();
    expect(
      within(details)
        .getByRole("link", { name: "Roadmap" })
        .getAttribute("href"),
    ).toBe("/p/n2");
    await user.click(
      screen.getByRole("button", { name: "Move “Launch plan” to the trash" }),
    );
    expect(api.trash).toHaveBeenCalledWith("n1");
    expect(
      await screen.findByText("Moved “Launch plan” to the trash."),
    ).toBeDefined();
  });

  it("follows a shortcut to its target", async () => {
    const user = renderAt("/drive", fakeApi());
    await user.click(
      await screen.findByRole("button", { name: "Plan shortcut" }),
    );
    const where = await screen.findByTestId("where");
    expect(where.textContent).toContain("/p/n1");
  });

  it("shows folder breadcrumbs and makes folders there", async () => {
    const api = fakeApi();
    const user = renderAt("/drive/s/s1/f/f1", api);
    const crumbs = await screen.findByRole("navigation", {
      name: "Folder path",
    });
    const folder = await within(crumbs).findByRole("link", { name: "Docs" });
    expect(folder.getAttribute("aria-current")).toBe("page");
    await user.type(screen.getByLabelText("New folder"), "Specs");
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(api.createFolder).toHaveBeenCalledWith("s1", "f1", "Specs");
  });

  it("lists items shared with me", async () => {
    renderAt("/drive/shared-with-me", fakeApi());
    expect(
      await screen.findByRole("link", { name: "Shared spec" }),
    ).toBeDefined();
  });

  it("restores, deletes for good after a confirmation, and empties the trash", async () => {
    const api = fakeApi();
    const user = renderAt("/drive/s/s1/trash", api);
    expect(
      await screen.findByText(/stay in the trash for 30 days/),
    ).toBeDefined();
    await user.click(
      screen.getByRole("button", { name: "Restore “Old notes”" }),
    );
    expect(api.restore).toHaveBeenCalledWith("t1");
    await user.click(screen.getByRole("button", { name: "Delete for good" }));
    expect(api.purge).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Delete “Old notes” for good" }),
    );
    expect(api.purge).toHaveBeenCalledWith("t1");
    await user.click(screen.getByRole("button", { name: "Empty trash" }));
    await user.click(
      screen.getByRole("button", { name: "Delete 1 item for good" }),
    );
    expect(await screen.findByText("Deleted 1 item for good.")).toBeDefined();
  });

  it("saves quota and retention settings and prunes versions", async () => {
    const api = fakeApi();
    const user = renderAt("/drive/s/s2/settings", api);
    const quota = await screen.findByLabelText("Quota in GB");
    await user.clear(quota);
    await user.type(quota, "2");
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    expect(api.updateSpace).toHaveBeenCalledWith(
      "s2",
      undefined,
      expect.objectContaining({ quota_bytes: 2 * 1024 ** 3, weekly_weeks: 12 }),
    );
    expect(await screen.findByText("Settings saved.")).toBeDefined();
    await user.clear(screen.getByLabelText("Days in the trash"));
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    expect(
      screen.getByLabelText("Days in the trash").getAttribute("aria-invalid"),
    ).toBe("true");
    await user.click(
      screen.getByRole("button", { name: "Apply retention now" }),
    );
    expect(await screen.findByText("Removed 4 old versions.")).toBeDefined();
  });

  it("searches with filters kept in the URL", async () => {
    const api = fakeApi();
    const user = renderAt("/drive/search", api);
    await user.type(await screen.findByLabelText("Title contains"), "plan");
    await user.click(screen.getByRole("checkbox", { name: "Page" }));
    await user.selectOptions(screen.getByLabelText("In"), "s2");
    await user.click(screen.getByRole("button", { name: "Search" }));
    expect(screen.getByTestId("where").textContent).toContain(
      "/drive/search?q=plan&kind=page&in=s2",
    );
    expect(
      await screen.findByRole("link", { name: "Launch plan" }),
    ).toBeDefined();
    expect(api.find).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "plan",
        kinds: ["page"],
        location: "s2",
      }),
    );
  });
});
