// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";

import { descendants, movePage } from "./tree";
import type { MoveTarget, PageMeta, Person, SearchHit, Space } from "./types";

/** Everything the shell renders, as one immutable snapshot. */
export interface WorkspaceSnapshot {
  readonly name: string;
  readonly me: Person;
  readonly people: readonly Person[];
  readonly spaces: readonly Space[];
  /** Pages that are not in the trash. */
  readonly pages: readonly PageMeta[];
  /** True while the pages are still on their way from the server. */
  readonly loading?: boolean;
  /** The last change or load that failed, until it is dismissed. */
  readonly problem?: StoreProblem | undefined;
}

/** A failed load or change, as the shell shows it. */
export interface StoreProblem {
  /** Safe to show as is. */
  readonly message: string;
  /** The server's request ID, to quote when reporting it. */
  readonly requestId?: string;
}

/** Who is signed in, as the server knows them. */
export interface SignedInUser extends Person {
  readonly id: string;
}

/**
 * The workspace port. The mock adapter below serves the Northstar demo;
 * the HTTP adapter (`./http`) serves the drive of a Vellum server.
 *
 * Reads are synchronous snapshots. Changes apply at once and settle in the
 * background; one that fails is undone and shows as the snapshot's
 * `problem`. Creating a page waits for its ID, and search may ask the
 * server, so those two answer with promises that never reject.
 */
export interface WorkspaceStore {
  getSnapshot(): WorkspaceSnapshot;
  subscribe(listener: () => void): () => void;
  /** The new page, or `undefined` when it could not be created. */
  createPage(input: {
    spaceId: string;
    parentId: string | undefined;
    title?: string;
    /** The first body, in Vellum Markdown; the server parses it into the page. */
    body?: string;
  }): Promise<PageMeta | undefined>;
  renamePage(id: string, title: string): void;
  toggleStar(id: string): void;
  movePage(id: string, target: MoveTarget): void;
  /** Moves a page and its subtree to the trash. */
  trashPage(id: string): void;
  /** The body a page's collab document starts from, if it has never been opened. */
  seedFor(id: string): JSONContent | undefined;
  search(query: string): Promise<SearchHit[]>;
  /** Pages that mention a handle, newest first: the inbox. */
  mentions(handle: string): Promise<SearchHit[]>;
  /** Clears the snapshot's `problem`. */
  dismissProblem(): void;
  /**
   * Tells the store who is signed in, when the app runs against a server:
   * it loads that person's spaces, or empties when nobody is.
   */
  setUser?(user: SignedInUser | undefined): void;
}

/** Seed data for an in-memory store. */
export interface WorkspaceSeed {
  readonly name: string;
  readonly me: Person;
  readonly people: readonly Person[];
  readonly spaces: readonly Space[];
  readonly pages: readonly PageMeta[];
  readonly bodies: Readonly<Record<string, JSONContent>>;
}

/** Whether a document mentions a handle anywhere. */
export function mentionsHandle(node: JSONContent | undefined, handle: string): boolean {
  if (!node) {
    return false;
  }
  if (node.type === "mention" && node.attrs?.["handle"] === handle) {
    return true;
  }
  return (node.content ?? []).some((child) => mentionsHandle(child, handle));
}

/** Plain text of a document, for search. */
export function plainText(node: JSONContent | undefined): string {
  if (!node) {
    return "";
  }
  if (node.type === "mention") {
    return `@${String(node.attrs?.["handle"] ?? "")}`;
  }
  const own = node.text ?? "";
  const attributes = node.attrs ?? {};
  const atom = ["key", "target", "label", "summary", "title"]
    .map((k): unknown => attributes[k])
    .filter((v): v is string => typeof v === "string" && v !== "")
    .join(" ");
  const children = (node.content ?? []).map((child) => plainText(child)).join(" ");
  return [own, atom, children].filter(Boolean).join(" ");
}

function snippet(text: string, query: string): string {
  const at = text.toLowerCase().indexOf(query);
  if (at === -1) {
    return "";
  }
  const start = Math.max(0, at - 40);
  const end = Math.min(text.length, at + query.length + 60);
  return `${start > 0 ? "…" : ""}${text.slice(start, end).trim()}${end < text.length ? "…" : ""}`;
}

/** Makes a page ULID: 48-bit time plus 80 random bits, Crockford base32. */
export function newPageId(now = Date.now(), random: () => number = Math.random): string {
  const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let time = "";
  let rest = now;
  for (let index = 0; index < 10; index += 1) {
    time = (alphabet[rest % 32] ?? "0") + time;
    rest = Math.floor(rest / 32);
  }
  let tail = "";
  for (let index = 0; index < 16; index += 1) {
    tail += alphabet[Math.floor(random() * 32)] ?? "0";
  }
  return time + tail;
}

/** An in-memory store. Used for the demo workspace and in tests. */
export function createMemoryStore(seed: WorkspaceSeed, clock: () => Date = () => new Date()) {
  let snapshot: WorkspaceSnapshot = {
    name: seed.name,
    me: seed.me,
    people: seed.people,
    spaces: seed.spaces,
    pages: seed.pages,
  };
  const bodies = new Map(Object.entries(seed.bodies));
  const listeners = new Set<() => void>();

  const setPages = (pages: readonly PageMeta[]) => {
    snapshot = { ...snapshot, pages };
    for (const listener of listeners) {
      listener();
    }
  };
  const patch = (id: string, change: (page: PageMeta) => PageMeta) => {
    setPages(snapshot.pages.map((p) => (p.id === id ? change(p) : p)));
  };
  const stamp = () => clock().toISOString();

  const store: WorkspaceStore = {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    createPage({ spaceId, parentId, title = "", body }) {
      const row = snapshot.pages.filter((p) => p.spaceId === spaceId && p.parentId === parentId);
      const page: PageMeta = {
        id: newPageId(clock().getTime()),
        spaceId,
        parentId,
        position: Math.max(0, ...row.map((p) => p.position)) + 1,
        title,
        icon: "📄",
        status: "draft",
        owner: snapshot.me.handle,
        updatedAt: stamp(),
        starred: false,
      };
      if (body !== undefined) bodies.set(page.id, paragraphsOf(body));
      setPages([...snapshot.pages, page]);
      return Promise.resolve(page);
    },
    renamePage(id, title) {
      patch(id, (p) => ({ ...p, title, updatedAt: stamp() }));
    },
    toggleStar(id) {
      patch(id, (p) => ({ ...p, starred: !p.starred }));
    },
    movePage(id, target) {
      setPages(movePage(snapshot.pages, id, target));
    },
    trashPage(id) {
      const gone = descendants(snapshot.pages, id);
      setPages(snapshot.pages.filter((p) => !gone.has(p.id)));
    },
    seedFor: (id) => bodies.get(id),
    search: (raw) => Promise.resolve(search(raw)),
    mentions: (handle) => Promise.resolve(mentions(handle)),
    dismissProblem() {
      // The memory store never fails.
    },
  };
  function search(raw: string): SearchHit[] {
    const query = raw.trim().toLowerCase();
    if (query === "") {
      return [];
    }
    const hits: SearchHit[] = [];
    for (const page of snapshot.pages) {
      const body = plainText(bodies.get(page.id));
      const inTitle = page.title.toLowerCase().includes(query);
      const text = snippet(body, query);
      if (inTitle || text !== "") {
        hits.push({ page, snippet: text });
      }
    }
    return hits.toSorted(
      (a, b) =>
        Number(b.page.title.toLowerCase().includes(query)) -
          Number(a.page.title.toLowerCase().includes(query)) ||
        a.page.title.localeCompare(b.page.title),
    );
  }
  function mentions(handle: string): SearchHit[] {
    return snapshot.pages
      .filter((page) => mentionsHandle(bodies.get(page.id), handle))
      .toSorted((a, b) => b.updatedAt.localeCompare(a.updatedAt))
      .map((page) => ({ page, snippet: snippet(plainText(bodies.get(page.id)), `@${handle}`) }));
  }
  return store;
}

/** Markdown as plain paragraphs, one per non-empty line: enough for demo pages. */
function paragraphsOf(markdown: string): JSONContent {
  return {
    type: "doc",
    content: markdown
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line !== "")
      .map((text) => ({ type: "paragraph", content: [{ type: "text", text }] })),
  };
}
