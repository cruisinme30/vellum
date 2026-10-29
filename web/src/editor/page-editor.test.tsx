// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen } from "@testing-library/react";
import type { Editor } from "@tiptap/core";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PAGE } from "../data/mock/ids";
import { NORTHSTAR_BODIES } from "../data/mock/northstar-pages";

import { createLocalConnector } from "./collab/local-connector";
import type { CollabConnector, LiveConnection, LiveState } from "./collab/provider";
import { body, h, p, wiki } from "./content";
import { PageEditor, type PageEditorProperties } from "./page-editor";
import { memoryChannels, resolveGateway, settle } from "./test-editor";

const user = { name: "Priya", color: "#1a73e8" };
const seed = body(h(1, "Plan"), p("See ", wiki("Gateway"), " and ", wiki("Nowhere")));

function editorProperties(
  connector: CollabConnector,
  overrides: Partial<PageEditorProperties> = {},
): PageEditorProperties {
  return {
    pageId: "page-1",
    spaceId: "space-1",
    seed,
    connector,
    user,
    resolvePage: resolveGateway,
    onOpenPage: vi.fn<(id: string) => void>(),
    ...overrides,
  };
}

/** Tiptap keeps its editor on the view's DOM node. */
function editorOf(element: HTMLElement | undefined): Editor {
  const editor = (element as (HTMLElement & { editor?: Editor }) | undefined)?.editor;
  if (!editor) {
    throw new Error("no editor on the element");
  }
  return editor;
}

/** A local session with a server state the test drives. */
function serverConnector(initial: LiveState) {
  const { factory } = memoryChannels();
  const local = createLocalConnector({ channel: factory });
  let state = initial;
  const listeners = new Set<() => void>();
  const live: LiveConnection = {
    getSnapshot: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    done: vi.fn(() => Promise.resolve()),
  };
  const connector: CollabConnector = {
    open: (id, space) => ({ ...local.open(id, space), live }),
  };
  const set = (next: Partial<LiveState>) => {
    state = { ...state, ...next };
    act(() => {
      for (const listener of listeners) {
        listener();
      }
    });
  };
  return { connector, set, live };
}

const notice = () => screen.getByRole("status");

const editors = () => screen.getAllByRole("textbox", { name: "Page body" });

afterEach(() => {
  vi.restoreAllMocks();
});

describe("PageEditor", () => {
  it("shows a loading state, then the toolbar and the seeded page", async () => {
    const { factory } = memoryChannels();
    render(<PageEditor {...editorProperties(createLocalConnector({ channel: factory }))} />);
    expect(screen.getByRole("status").textContent).toBe("Opening the page…");
    await settle();
    expect(screen.getByRole("toolbar", { name: "Formatting" })).toBeDefined();
    const [editor] = editors();
    expect(editor?.getAttribute("aria-multiline")).toBe("true");
    expect(editor?.querySelector("h1")?.textContent).toBe("Plan");
  });

  it("seeds the content once when two sessions open the same page", async () => {
    const { factory } = memoryChannels();
    const connector = createLocalConnector({ channel: factory });
    render(
      <>
        <PageEditor {...editorProperties(connector)} />
        <PageEditor {...editorProperties(connector)} />
      </>,
    );
    await settle();
    for (const editor of editors()) {
      expect(editor.querySelectorAll("h1")).toHaveLength(1);
    }
  });

  it("converges two sessions after an edit in one of them", async () => {
    const { factory } = memoryChannels();
    const connector = createLocalConnector({ channel: factory });
    render(
      <>
        <PageEditor {...editorProperties(connector)} />
        <PageEditor {...editorProperties(connector, { seed: undefined })} />
      </>,
    );
    await settle();
    const [first, second] = editors();
    expect(second?.querySelector("h1")?.textContent).toBe("Plan");
    const heading = first?.querySelector("h1");
    act(() => {
      heading?.append(" v2");
    });
    await settle();
    expect(first?.querySelector("h1")?.textContent).toBe("Plan v2");
    expect(second?.querySelector("h1")?.textContent).toBe("Plan v2");
  });

  it("opens a resolved wiki link and ignores a missing one", async () => {
    const { factory } = memoryChannels();
    const onOpenPage = vi.fn<(id: string) => void>();
    render(
      <PageEditor
        {...editorProperties(createLocalConnector({ channel: factory }), { onOpenPage })}
      />,
    );
    await settle();
    // jsdom has no layout, so ProseMirror cannot map a click to a position: call the prop.
    const editor = editorOf(editors()[0]);
    const clickOn = (target: string) => {
      let handled = false;
      editor.state.doc.descendants((node, pos) => {
        if (node.attrs["target"] === target) {
          handled =
            editor.view.someProp("handleClickOn", (handler) =>
              handler(editor.view, pos, node, pos, new MouseEvent("click"), true),
            ) ?? false;
        }
      });
      return handled;
    };
    expect(clickOn("Gateway")).toBe(true);
    expect(onOpenPage).toHaveBeenCalledWith("page-gateway");
    onOpenPage.mockClear();
    expect(clickOn("Nowhere")).toBe(true);
    expect(onOpenPage).not.toHaveBeenCalled();
    const heading = editor.state.doc.firstChild;
    expect(
      heading &&
        editor.view.someProp("handleClickOn", (handler) =>
          handler(editor.view, 1, heading, 0, new MouseEvent("click"), true),
        ),
    ).toBeFalsy();
  });

  it("keeps the browser from following a wiki link's href", async () => {
    const { factory } = memoryChannels();
    render(<PageEditor {...editorProperties(createLocalConnector({ channel: factory }))} />);
    await settle();
    const anchor = editors()[0]?.querySelector("a.vl-wikilink");
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });
    act(() => {
      anchor?.dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(true);
  });

  it("leaves clicks on ordinary text alone", async () => {
    const { factory } = memoryChannels();
    const onOpenPage = vi.fn<(id: string) => void>();
    render(
      <PageEditor
        {...editorProperties(createLocalConnector({ channel: factory }), { onOpenPage })}
      />,
    );
    await settle();
    const heading = editors()[0]?.querySelector("h1");
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });
    act(() => {
      heading?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      heading?.dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(false);
    expect(onOpenPage).not.toHaveBeenCalled();
  });

  it("opens a new session for another page and closes the old one", async () => {
    const { factory, open } = memoryChannels();
    const connector = createLocalConnector({ channel: factory });
    const { rerender, unmount } = render(<PageEditor {...editorProperties(connector)} />);
    await settle();
    rerender(
      <PageEditor
        {...editorProperties(connector, { pageId: "page-2", seed: body(h(1, "Other")) })}
      />,
    );
    await settle();
    expect(open()).toBe(1);
    expect(editors()[0]?.querySelector("h1")?.textContent).toBe("Other");
    unmount();
    expect(open()).toBe(0);
  });

  it("does not bind a session that finished opening after unmount", async () => {
    const { factory, open } = memoryChannels();
    const { unmount } = render(
      <PageEditor {...editorProperties(createLocalConnector({ channel: factory }))} />,
    );
    unmount();
    await settle();
    expect(open()).toBe(0);
  });

  it("loads the PRD page, which starts with a facts block", async () => {
    const { factory } = memoryChannels();
    render(
      <PageEditor
        {...editorProperties(createLocalConnector({ channel: factory }), {
          seed: NORTHSTAR_BODIES[PAGE.prd],
        })}
      />,
    );
    await settle();
    expect(editors()[0]?.querySelector(".vl-facts")).not.toBeNull();
    expect(editors()[0]?.querySelector(".vl-toc")).not.toBeNull();
  });

  describe("on a server", () => {
    it("opens the room named by the page's space", async () => {
      const { factory } = memoryChannels();
      const local = createLocalConnector({ channel: factory });
      const open = vi.fn((id: string, space: string) => local.open(id, space));
      render(
        <PageEditor {...editorProperties({ open }, { pageId: "01JPAGE", spaceId: "01JSPACE" })} />,
      );
      await settle();
      expect(open).toHaveBeenCalledWith("01JPAGE", "01JSPACE");
    });

    it("saves a version on Done and on Ctrl+S, and says when it is saved", async () => {
      const { connector, live } = serverConnector({
        status: "online",
        canWrite: true,
        peers: new Map(),
      });
      const onControls = vi.fn();
      render(<PageEditor {...editorProperties(connector, { onControls })} />);
      await settle();
      const done = screen.getByRole("button", { name: "Done" });
      expect(done.hasAttribute("disabled")).toBe(false);
      act(() => {
        editorOf(editors()[0]).commands.insertContent("More");
      });
      expect(screen.getByText("Unsaved changes")).toBeDefined();

      act(() => {
        done.click();
      });
      await settle();
      expect(live.done).toHaveBeenCalledTimes(1);
      expect(screen.getByText("Saved")).toBeDefined();

      const body = editors()[0];
      act(() => {
        body?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true }),
        );
      });
      await settle();
      expect(live.done).toHaveBeenCalledTimes(2);
      expect(onControls).toHaveBeenLastCalledWith(expect.objectContaining({ canWrite: true }));
    });

    it("is read-only with a notice until the server lets this person write", async () => {
      const { connector, set } = serverConnector({
        status: "online",
        canWrite: false,
        peers: new Map(),
      });
      render(<PageEditor {...editorProperties(connector)} />);
      await settle();
      const editor = editorOf(editors()[0]);
      expect(editor.isEditable).toBe(false);
      expect(notice().textContent).toBe("You can view this page but not edit it.");
      set({ canWrite: true });
      expect(editor.isEditable).toBe(true);
      expect(notice().textContent).toBe("");
    });

    it("keeps editing offline and says the changes will sync", async () => {
      const { connector, set } = serverConnector({
        status: "online",
        canWrite: true,
        peers: new Map(),
      });
      render(<PageEditor {...editorProperties(connector)} />);
      await settle();
      set({ status: "offline" });
      expect(editorOf(editors()[0]).isEditable).toBe(true);
      expect(notice().textContent).toContain("Offline.");
      set({ status: "connecting" });
      expect(notice().textContent).toBe("Connecting to the server…");
    });

    it("stops editing when access is revoked", async () => {
      const { connector, set } = serverConnector({
        status: "online",
        canWrite: true,
        peers: new Map(),
      });
      render(<PageEditor {...editorProperties(connector)} />);
      await settle();
      set({ status: "revoked", canWrite: false });
      expect(editorOf(editors()[0]).isEditable).toBe(false);
      expect(notice().textContent).toContain("access to this page was removed");
    });
  });
});
