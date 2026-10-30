// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Test helpers for the block editor: a headless editor with every Vellum extension.
import { act, render } from "@testing-library/react";
import { Editor, type JSONContent } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { DecorationSet } from "@tiptap/pm/view";
import { EditorContent, useEditor, type ReactNodeViewProps } from "@tiptap/react";
import { createElement, createRef, useEffect, type ComponentType, type ReactNode } from "react";

import { ServerApisProvider } from "../data/server-apis";
import en from "../i18n/en.ftl?raw";
import { createTranslator } from "../i18n/i18n";
import type { RenderApi } from "../render/api";

import type { ChipSources } from "./chips/items";
import type { ChannelFactory } from "./collab/local-connector";
import { vellumExtensions } from "./extensions";
import type { ResolvePage } from "./nodes/inline";

/** The English translator the app ships. */
export const english = createTranslator("en", en);

/** Resolves only the page titled "Gateway". */
export const resolveGateway: ResolvePage = (target) =>
  target === "Gateway" ? { id: "page-gateway", title: "Gateway" } : undefined;

/** Options for {@link createTestEditor}. */
export interface TestEditorOptions {
  readonly content?: JSONContent | string;
  readonly resolvePage?: ResolvePage;
}

/** A Vellum editor with history on, as in unit tests. */
export function createTestEditor(options: TestEditorOptions = {}): Editor {
  return new Editor({
    element: document.createElement("div"),
    extensions: vellumExtensions({
      t: english,
      resolvePage: options.resolvePage ?? resolveGateway,
      history: true,
    }),
    content: options.content ?? "",
  });
}

/** Types text at the cursor the way a keyboard does, so input rules run. */
export function typeText(editor: Editor, text: string): void {
  for (const character of text) {
    const { from, to } = editor.state.selection;
    const view = editor.view;
    const handled = view.someProp("handleTextInput", (handler) =>
      handler(view, from, to, character, () => view.state.tr.insertText(character, from, to)),
    );
    if (handled !== true) {
      view.dispatch(view.state.tr.insertText(character, from, to));
    }
  }
}

/** The node type names of the document's top-level blocks. */
export function topLevelTypes(editor: Editor): string[] {
  return editor.state.doc.children.map((node) => node.type.name);
}

/** Props of {@link EditorHarness}. */
interface EditorHarnessProperties {
  readonly content: JSONContent | string;
  readonly editable?: boolean;
  readonly onEditor?: (editor: Editor) => void;
  readonly chips?: ChipSources | undefined;
}

/** A React-mounted Vellum editor, so node views render through portals. */
function EditorHarness({ content, editable = true, onEditor, chips }: EditorHarnessProperties) {
  const editor = useEditor({
    extensions: vellumExtensions({ t: english, resolvePage: resolveGateway, history: true, chips }),
    content,
    editable,
  });
  useEffect(() => {
    onEditor?.(editor);
  }, [editor, onEditor]);
  return createElement(EditorContent, { editor });
}

/** Renders a React editor, waits for its node views and hands back the Tiptap instance. */
export async function renderEditor(
  content: JSONContent | string,
  options: {
    readonly editable?: boolean;
    readonly chips?: ChipSources;
    /** Providers around the editor, such as the page's files. */
    readonly wrapper?: ComponentType<{ children: ReactNode }>;
    /** A server renderer, for blocks drawn by the server. */
    readonly render?: RenderApi;
  } = {},
): Promise<{ editor: Editor; unmount: () => void }> {
  let current: Editor | undefined;
  const harness = createElement(EditorHarness, {
    content,
    editable: options.editable ?? true,
    chips: options.chips,
    onEditor: (editor: Editor) => {
      current = editor;
    },
  });
  const { unmount } = render(
    options.render === undefined
      ? harness
      : createElement(ServerApisProvider, { apis: { render: options.render } }, harness),
    options.wrapper === undefined ? {} : { wrapper: options.wrapper },
  );
  // React node views render in a microtask after the editor mounts.
  await act(async () => {
    await Promise.resolve();
  });
  if (!current) {
    throw new Error("The editor did not mount");
  }
  return { editor: current, unmount };
}

/** An in-memory BroadcastChannel stand-in: messages reach the other channels of a name. */
class MemoryChannel extends EventTarget {
  constructor(
    private readonly peers: Set<MemoryChannel>,
    private readonly log: unknown[],
  ) {
    super();
    peers.add(this);
  }

  postMessage(data: unknown): void {
    this.log.push(data);
    for (const peer of this.peers) {
      if (peer !== this) {
        queueMicrotask(() => {
          peer.dispatchEvent(new MessageEvent("message", { data }));
        });
      }
    }
  }

  close(): void {
    this.peers.delete(this);
  }
}

/** A channel factory whose channels live in memory, with every message posted. */
export function memoryChannels(): {
  factory: ChannelFactory;
  posted: unknown[];
  open: () => number;
} {
  const rooms = new Map<string, Set<MemoryChannel>>();
  const posted: unknown[] = [];
  const factory: ChannelFactory = (name) => {
    const peers = rooms.get(name) ?? new Set<MemoryChannel>();
    rooms.set(name, peers);
    return new MemoryChannel(peers, posted);
  };
  const open = () => [...rooms.values()].reduce((count, peers) => count + peers.size, 0);
  return { factory, posted, open };
}

/** Lets queued channel messages and promise callbacks run. */
export async function settle(): Promise<void> {
  await act(async () => {
    for (let round = 0; round < 5; round += 1) {
      await new Promise((resolve) => {
        setTimeout(resolve, 0);
      });
    }
  });
}

const noop = () => {
  // Detached: nothing to update.
};

/**
 * Props for rendering a node view component on its own, outside an editor
 * view, as ProseMirror does for a node that has just been detached.
 */
export function detachedViewProperties(editor: Editor, node: ProseMirrorNode): ReactNodeViewProps {
  return {
    editor,
    node,
    view: editor.view,
    // eslint-disable-next-line unicorn/no-useless-undefined -- a detached node has no position
    getPos: () => undefined,
    decorations: [],
    innerDecorations: DecorationSet.empty,
    selected: false,
    extension: editor.extensionManager.extensions[0],
    HTMLAttributes: {},
    updateAttributes: noop,
    deleteNode: noop,
    ref: createRef<HTMLElement>(),
  } as ReactNodeViewProps;
}
