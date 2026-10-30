// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The drive (`/api/v1/drive`): spaces, their items, trash, activity,
 * backlinks, search and quotas. Types come from the server's generated
 * client; writes go through the app's HTTP client, so they carry the CSRF
 * token and an idempotency key.
 */

import type {
  ActivityList,
  ActivityView,
  FindInput,
  MyDrive,
  NodeKindView,
  NodeList,
  NodeView,
  RecentList,
  Removed,
  SecretScanResult,
  SettingsView,
  SpaceList,
  SpaceView,
  TreeView,
  UsageView,
} from "../api/api.gen";
import type { Http, Query } from "../api/http";

/** Where to move, copy or put a shortcut to an item. */
export interface Place {
  readonly spaceId: string;
  /** The folder or page it goes into; the top of the space when absent. */
  readonly parentId?: string | undefined;
}

/** Search filters; every filter given must match (`drive.find`). */
export interface Filters {
  readonly title?: string | undefined;
  readonly kinds?: readonly NodeKindView[] | undefined;
  /** Modified on or after this day, `YYYY-MM-DD`. */
  readonly after?: string | undefined;
  /** Modified before the end of this day, `YYYY-MM-DD`. */
  readonly before?: string | undefined;
  /** A space or folder the item must be in, at any depth. */
  readonly location?: string | undefined;
  readonly starred?: boolean | undefined;
}

/** The drive operations the web app uses. */
export interface DriveApi {
  /** The signed-in person's My Drive; the server makes it on first visit. */
  readonly myDrive: () => Promise<SpaceView | undefined>;
  readonly sharedDrives: () => Promise<SpaceView[]>;
  /** Items in other people's My Drives shared with the person. */
  readonly sharedWithMe: () => Promise<NodeView[]>;
  readonly starred: () => Promise<NodeView[]>;
  readonly recent: () => Promise<NodeView[]>;
  /** The items in a folder, or at the top of a space. */
  readonly list: (spaceId: string, parentId?: string) => Promise<NodeView[]>;
  readonly get: (id: string) => Promise<NodeView>;
  /** The folders and pages of a space, for choosing a destination. */
  readonly containers: (
    spaceId: string,
  ) => Promise<{ items: NodeView[]; truncated: boolean }>;
  readonly trashList: (spaceId: string) => Promise<NodeView[]>;
  readonly activity: (id: string) => Promise<ActivityView[]>;
  readonly backlinks: (id: string) => Promise<NodeView[]>;
  readonly usage: (spaceId: string) => Promise<UsageView>;
  readonly find: (filters: Filters) => Promise<NodeView[]>;
  /** The item a shortcut points at. */
  readonly target: (id: string) => Promise<NodeView>;
  readonly move: (id: string, to: Place) => Promise<NodeView>;
  readonly copy: (id: string, to: Place) => Promise<NodeView>;
  readonly shortcut: (targetId: string, to: Place) => Promise<NodeView>;
  readonly createFolder: (
    spaceId: string,
    parentId: string | undefined,
    name: string,
  ) => Promise<NodeView>;
  readonly rename: (id: string, title: string) => Promise<NodeView>;
  readonly star: (id: string, starred: boolean) => Promise<void>;
  readonly trash: (id: string) => Promise<NodeView>;
  readonly restore: (id: string) => Promise<NodeView>;
  /** Deletes a trashed item for good. */
  readonly purge: (id: string) => Promise<void>;
  /** Deletes everything in a space's trash for good; returns how many. */
  readonly emptyTrash: (spaceId: string) => Promise<number>;
  readonly updateSpace: (
    spaceId: string,
    name: string | undefined,
    settings: SettingsView,
  ) => Promise<SpaceView>;
  /** Applies the space's version retention now; returns how many went. */
  readonly pruneVersions: (spaceId: string) => Promise<number>;
  /** The likely secrets in page text, and what a save of it would meet. */
  readonly scanSecrets?: (text: string) => Promise<SecretScanResult>;
}

const DRIVE = "/api/v1/drive";
const segment = encodeURIComponent;
const item = (id: string) => `${DRIVE}/items/${segment(id)}`;
const space = (id: string) => `${DRIVE}/spaces/${segment(id)}`;
/** The most items one destination tree asks for; the server's own cap. */
const TREE_LIMIT = 10_000;

/**
 * A `YYYY-MM-DD` day as the start of that day in the person's zone, as RFC
 * 3339.
 */
function startOf(day: string, plusDays = 0): string | undefined {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (match === null) return undefined;
  const date = new Date(
    Number(match[1]),
    Number(match[2]) - 1,
    Number(match[3]) + plusDays,
  );
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

/** The `drive.find` query for some filters; empty filters are left out. */
export function findQuery(filters: Filters): Query {
  const title = filters.title?.trim();
  return {
    title: title === "" ? undefined : title,
    kinds:
      filters.kinds === undefined || filters.kinds.length === 0
        ? undefined
        : filters.kinds,
    modified_after:
      filters.after === undefined ? undefined : startOf(filters.after),
    modified_before:
      filters.before === undefined ? undefined : startOf(filters.before, 1),
    location: filters.location === "" ? undefined : filters.location,
    starred: filters.starred === true ? true : undefined,
    limit: 200,
  } satisfies Partial<Record<keyof FindInput, unknown>>;
}

const placeBody = (to: Place) => ({
  space_id: to.spaceId,
  parent_id: to.parentId,
});

/** The drive over a Vellum server. */
export function createHttpDriveApi(http: Http): DriveApi {
  const items = async (path: string, query?: Query) => {
    const found = await http.get<NodeList>(path, query);
    return found.items;
  };
  return {
    scanSecrets: (text) =>
      http.send<SecretScanResult>("POST", "/api/v1/security/secrets/scan", {
        text,
      }),
    async myDrive() {
      const found = await http.get<MyDrive>(`${DRIVE}/my-drive`);
      return found.space ?? undefined;
    },
    async sharedDrives() {
      const found = await http.get<SpaceList>(`${DRIVE}/shared-drives`);
      return found.spaces.toSorted((a, b) => a.name.localeCompare(b.name));
    },
    sharedWithMe: () => items(`${DRIVE}/shared-with-me`),
    starred: () => items(`${DRIVE}/starred`),
    async recent() {
      const found = await http.get<RecentList>(`${DRIVE}/recent`, {
        limit: 50,
      });
      return found.items.map((entry) => entry.item);
    },
    list: (spaceId, parentId) =>
      items(`${space(spaceId)}/items`, { parent_id: parentId }),
    get: (id) => http.get<NodeView>(item(id)),
    async containers(spaceId) {
      const tree = await http.get<TreeView>(`${space(spaceId)}/tree`, {
        limit: TREE_LIMIT,
      });
      return {
        items: tree.items.filter(
          (node) => node.kind === "folder" || node.kind === "page",
        ),
        truncated: tree.truncated,
      };
    },
    trashList: (spaceId) => items(`${space(spaceId)}/trash`),
    async activity(id) {
      const found = await http.get<ActivityList>(`${item(id)}/activity`, {
        limit: 50,
      });
      return found.events;
    },
    backlinks: (id) => items(`${item(id)}/backlinks`),
    usage: (spaceId) => http.get<UsageView>(`${space(spaceId)}/usage`),
    find: (filters) => items(`${DRIVE}/search`, findQuery(filters)),
    target: (id) => http.get<NodeView>(`${item(id)}/target`),
    move: (id, to) =>
      http.send<NodeView>("POST", `${item(id)}/move`, placeBody(to)),
    copy: (id, to) =>
      http.send<NodeView>("POST", `${item(id)}/copy`, placeBody(to)),
    shortcut: (targetId, to) =>
      http.send<NodeView>("POST", `${space(to.spaceId)}/shortcuts`, {
        parent_id: to.parentId,
        target_id: targetId,
      }),
    createFolder: (spaceId, parentId, name) =>
      http.send<NodeView>("POST", `${space(spaceId)}/folders`, {
        parent_id: parentId,
        name,
      }),
    rename: (id, title) => http.send<NodeView>("PATCH", item(id), { title }),
    async star(id, starred) {
      await http.send("PUT", `${item(id)}/star`, { starred });
    },
    trash: (id) => http.send<NodeView>("POST", `${item(id)}/trash`, {}),
    restore: (id) => http.send<NodeView>("POST", `${item(id)}/restore`, {}),
    async purge(id) {
      await http.send("DELETE", item(id));
    },
    async emptyTrash(spaceId) {
      const removed = await http.send<Removed>(
        "DELETE",
        `${space(spaceId)}/trash`,
      );
      return removed.removed;
    },
    updateSpace: (spaceId, name, settings) =>
      http.send<SpaceView>("PATCH", space(spaceId), { name, settings }),
    async pruneVersions(spaceId) {
      const removed = await http.send<Removed>(
        "POST",
        `${space(spaceId)}/prune-versions`,
        {},
      );
      return removed.removed;
    },
  };
}

export {
  type ActorView,
  type ActivityView,
  type NodeKindView,
  type NodeView,
  type SettingsView,
  type SpaceView,
  type UsageView,
} from "../api/api.gen";
