// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { SearchPage } from "../api/api.gen";
import { ApiError, type Http } from "../api/http";

import type { SignedInUser, StoreProblem, WorkspaceSnapshot, WorkspaceStore } from "./store";
import { descendants, movePage } from "./tree";
import type { PageMeta, SearchHit, Space } from "./types";

/** `SpaceView` from `vellum-drive`. */
interface SpaceView {
  readonly id: string;
  readonly kind: "shared_drive" | "user_drive";
  readonly slug: string;
  readonly name: string;
}

/** `NodeView` from `vellum-drive`. */
interface NodeView {
  readonly id: string;
  readonly space_id: string;
  readonly kind: "folder" | "page" | "book" | "collection" | "shortcut";
  readonly parent_id?: string | null;
  readonly title: string;
  readonly modified: string;
  readonly created_by?: string | null;
}

interface NodeList {
  readonly items: readonly NodeView[];
}

/** `TreeView` from `vellum-drive`: a space's items, depth first in order. */
interface TreeList extends NodeList {
  /** True when the limit cut the tree short. */
  readonly truncated: boolean;
}

const DRIVE = "/api/v1/drive";
const SEARCH = "/api/v1/search";
/** Hits asked of the search service; several can fall in one page. */
const SEARCH_LIMIT = 50;
const segment = encodeURIComponent;
const item = (id: string) => `${DRIVE}/items/${segment(id)}`;

/** The most items one tree request asks for; the server's own cap. */
const TREE_LIMIT = 10_000;

/** Folders and pages make the tree; books, collections and shortcuts open elsewhere. */
const inTree = (node: NodeView) => node.kind === "page" || node.kind === "folder";

function problemOf(error: unknown): StoreProblem {
  if (error instanceof ApiError) {
    return error.requestId === undefined
      ? { message: error.message }
      : { message: error.message, requestId: error.requestId };
  }
  return { message: error instanceof Error ? error.message : String(error) };
}

const patch =
  (id: string, fields: Partial<PageMeta>) =>
  (pages: readonly PageMeta[]): readonly PageMeta[] =>
    pages.map((p) => (p.id === id ? { ...p, ...fields } : p));

/**
 * Undoes a change: the pages it touched go back to how they were (or go,
 * if it made them), and the pages it removed come back.
 */
function undo(
  pages: readonly PageMeta[],
  touched: ReadonlyMap<string, PageMeta | undefined>,
  removed: readonly PageMeta[],
): PageMeta[] {
  const restored = pages.flatMap((p) => {
    if (!touched.has(p.id)) return [p];
    const old = touched.get(p.id);
    return old === undefined ? [] : [old];
  });
  const present = new Set(restored.map((p) => p.id));
  return [...restored, ...removed.filter((p) => !present.has(p.id))];
}

function spaceOf(space: SpaceView): Space {
  return {
    id: space.id,
    slug: space.slug,
    name: space.name,
    icon: space.kind === "user_drive" ? "🏠" : "👥",
  };
}

/** Options for {@link createHttpWorkspaceStore}. */
export interface HttpWorkspaceOptions {
  /** The workspace's name in the sidebar. */
  readonly name?: string;
  /** Who is signed in when the app starts; loading starts at once. */
  readonly user?: SignedInUser | undefined;
}

/**
 * The workspace port over the drive's REST operations (`/api/v1/drive`,
 * issue #392). The snapshot holds the signed-in person's My Drive, which
 * the server makes with the account, and the shared drives they can open,
 * each with its whole folder and page tree. Changes show at once and settle with the server;
 * one the server refuses is undone and shows as the snapshot's `problem`.
 *
 * Page bodies travel over the live-editing socket, never through here
 * (ADR 0010), so `seedFor` has nothing to give.
 */
export function createHttpWorkspaceStore(
  http: Http,
  options: HttpWorkspaceOptions = {},
): WorkspaceStore {
  const name = options.name ?? "Vellum";
  const listeners = new Set<() => void>();
  let user: SignedInUser | undefined;
  /** Bumped on every load, so an answer for an earlier person is dropped. */
  let generation = 0;
  let snapshot: WorkspaceSnapshot = emptyFor();

  function emptyFor(who?: SignedInUser): WorkspaceSnapshot {
    const me =
      who === undefined ? { handle: "", name: "" } : { handle: who.handle, name: who.name };
    return {
      name,
      me,
      people: who === undefined ? [] : [me],
      spaces: [],
      pages: [],
      loading: who !== undefined,
    };
  }

  const set = (next: WorkspaceSnapshot) => {
    snapshot = next;
    for (const listener of listeners) listener();
  };
  const setPages = (change: (pages: readonly PageMeta[]) => readonly PageMeta[]) => {
    set({ ...snapshot, pages: change(snapshot.pages) });
  };
  const report = (error: unknown) => {
    set({ ...snapshot, problem: problemOf(error) });
  };

  const metaOf = (node: NodeView, position: number, starred: boolean): PageMeta => {
    const folder = node.kind === "folder";
    return {
      id: node.id,
      spaceId: node.space_id,
      parentId: node.parent_id ?? undefined,
      position,
      title: node.title,
      icon: folder ? "📁" : "📄",
      // The drive keeps no status; front matter is not in its views.
      status: "draft",
      owner: user !== undefined && node.created_by === user.id ? user.handle : "",
      updatedAt: node.modified,
      starred,
      ...(folder ? { folder } : {}),
    };
  };

  const myDrive = async (): Promise<SpaceView> => {
    const mine = await http.get<{ space?: SpaceView | null }>(`${DRIVE}/my-drive`);
    if (mine.space === undefined || mine.space === null) {
      // The server makes a My Drive with every account, so this is a fault.
      throw new Error("The server returned no My Drive.");
    }
    return mine.space;
  };

  /** Turns depth-first items into pages, numbering each list of siblings from 1. */
  const pagesOf = (items: readonly NodeView[], starred: ReadonlySet<string>): PageMeta[] => {
    const kept = new Set<string>();
    const counts = new Map<string | undefined, number>();
    const out: PageMeta[] = [];
    for (const node of items) {
      const parent = node.parent_id ?? undefined;
      // Only folders and pages make the tree, and nothing under what it leaves out.
      if (!inTree(node) || (parent !== undefined && !kept.has(parent))) continue;
      kept.add(node.id);
      const position = (counts.get(parent) ?? 0) + 1;
      counts.set(parent, position);
      out.push(metaOf(node, position, starred.has(node.id)));
    }
    return out;
  };

  /** A space's tree, level by level: slower, but never cut short. */
  const treeByLevel = async (spaceId: string): Promise<NodeView[]> => {
    const out: NodeView[] = [];
    let level: (string | undefined)[] = [undefined];
    while (level.length > 0) {
      const lists = await Promise.all(
        level.map((parent) =>
          http.get<NodeList>(`${DRIVE}/spaces/${segment(spaceId)}/items`, { parent_id: parent }),
        ),
      );
      const found = lists.flatMap((list) => list.items.filter((n) => inTree(n)));
      out.push(...found);
      level = found.map((node) => node.id);
    }
    return out;
  };

  /** A space's folders and pages in one request, or level by level when it is too big. */
  const tree = async (spaceId: string, starred: ReadonlySet<string>): Promise<PageMeta[]> => {
    const whole = await http.get<TreeList>(`${DRIVE}/spaces/${segment(spaceId)}/tree`, {
      limit: TREE_LIMIT,
    });
    const items = whole.truncated ? await treeByLevel(spaceId) : whole.items;
    return pagesOf(items, starred);
  };

  const load = async (who: SignedInUser) => {
    generation += 1;
    const mine = generation;
    set(emptyFor(who));
    try {
      const [own, shared, starredList] = await Promise.all([
        myDrive(),
        http.get<{ spaces: readonly SpaceView[] }>(`${DRIVE}/shared-drives`),
        http.get<NodeList>(`${DRIVE}/starred`),
      ]);
      const spaces = [own, ...shared.spaces.filter((s) => s.id !== own.id)];
      const starred = new Set(starredList.items.map((n) => n.id));
      const trees = await Promise.all(spaces.map((s) => tree(s.id, starred)));
      if (mine !== generation) return;
      set({
        ...snapshot,
        spaces: spaces.map((s) => spaceOf(s)),
        pages: trees.flat(),
        loading: false,
      });
    } catch (error) {
      if (mine !== generation) return;
      set({ ...snapshot, loading: false, problem: problemOf(error) });
    }
  };

  /**
   * Applies a change now and sends it; if the server refuses, puts back
   * the pages it touched as they were and reports why.
   */
  const change = <T>(
    apply: (pages: readonly PageMeta[]) => readonly PageMeta[],
    request: () => Promise<T>,
    settle?: (answer: T) => void,
  ) => {
    const before = new Map(snapshot.pages.map((p) => [p.id, p]));
    setPages(apply);
    const touched = new Map<string, PageMeta | undefined>();
    for (const page of snapshot.pages) {
      if (before.get(page.id) !== page) touched.set(page.id, before.get(page.id));
    }
    const kept = new Set(snapshot.pages.map((p) => p.id));
    const removed = [...before.values()].filter((p) => !kept.has(p.id));
    request().then(
      (answer) => settle?.(answer),
      (error: unknown) => {
        setPages((pages) => undo(pages, touched, removed));
        report(error);
      },
    );
  };

  const settleNode = (node: NodeView) => {
    setPages(patch(node.id, { title: node.title, updatedAt: node.modified }));
  };

  const store: WorkspaceStore = {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async createPage({ spaceId, parentId, title = "" }) {
      try {
        const node = await http.send<NodeView>(
          "POST",
          `${DRIVE}/spaces/${segment(spaceId)}/pages`,
          { parent_id: parentId, title },
        );
        const row = snapshot.pages.filter((p) => p.spaceId === spaceId && p.parentId === parentId);
        const page = metaOf(node, Math.max(0, ...row.map((p) => p.position)) + 1, false);
        setPages((pages) => [...pages, page]);
        return page;
      } catch (error) {
        report(error);
        return;
      }
    },
    renamePage(id, title) {
      change(
        patch(id, { title }),
        () => http.send<NodeView>("PATCH", item(id), { title }),
        settleNode,
      );
    },
    toggleStar(id) {
      const page = snapshot.pages.find((p) => p.id === id);
      if (page === undefined) return;
      const starred = !page.starred;
      change(patch(id, { starred }), () => http.send("PUT", `${item(id)}/star`, { starred }));
    },
    movePage(id, target) {
      if (!snapshot.pages.some((p) => p.id === id)) return;
      change(
        (pages) => movePage(pages, id, target),
        () =>
          http.send("POST", `${item(id)}/move`, {
            space_id: target.spaceId,
            parent_id: target.parentId,
            index: target.index,
          }),
      );
    },
    trashPage(id) {
      const gone = descendants(snapshot.pages, id);
      change(
        (pages) => pages.filter((p) => !gone.has(p.id)),
        () => http.send("POST", `${item(id)}/trash`),
      );
    },
    seedFor() {
      return;
    },
    async search(raw) {
      const q = raw.trim();
      if (q === "") return [];
      // Only what the tree holds opens here: shared-with-me items have no space in it yet.
      const known = new Map(snapshot.pages.map((p) => [p.id, p]));
      try {
        return await searchText(http, q, known);
      } catch (error) {
        if (!searchMissing(error)) {
          report(error);
          return [];
        }
      }
      try {
        return await searchTitles(http, q, known);
      } catch (error) {
        report(error);
        return [];
      }
    },
    // The server has no mentions index yet.
    mentions: () => Promise.resolve([]),
    dismissProblem() {
      if (snapshot.problem === undefined) return;
      set({ ...snapshot, problem: undefined });
    },
    setUser(next) {
      if (next?.id === user?.id) return;
      user = next;
      if (next === undefined) {
        generation += 1;
        set(emptyFor());
        return;
      }
      void load(next);
    },
    refresh() {
      if (user !== undefined) void load(user);
    },
  };
  store.setUser?.(options.user);
  return store;
}

/**
 * Full-text search: the best block of each page, best page first, with the
 * block's excerpt as the snippet.
 */
async function searchText(
  http: Http,
  q: string,
  known: ReadonlyMap<string, PageMeta>,
): Promise<SearchHit[]> {
  const found = await http.get<SearchPage>(SEARCH, { q, kinds: "page", limit: SEARCH_LIMIT });
  const hits = new Map<string, SearchHit>();
  for (const hit of found.hits) {
    const page = known.get(hit.doc);
    if (page === undefined || hits.has(page.id)) continue;
    const snippet = hit.snippet
      .map((fragment) => fragment.text)
      .join("")
      .trim();
    hits.set(page.id, { page, snippet });
  }
  return [...hits.values()];
}

/** Title search on the drive, for a server that runs no search service. */
async function searchTitles(
  http: Http,
  title: string,
  known: ReadonlyMap<string, PageMeta>,
): Promise<SearchHit[]> {
  const found = await http.get<NodeList>(`${DRIVE}/search`, { title, limit: SEARCH_LIMIT });
  return found.items.flatMap((node): SearchHit[] => {
    const page = known.get(node.id);
    return page === undefined ? [] : [{ page, snippet: "" }];
  });
}

/** Whether a search failed because the server has no search service. */
const searchMissing = (error: unknown) =>
  error instanceof ApiError && (error.status === 404 || error.status === 501);
