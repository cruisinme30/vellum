// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";
import { describe, expect, it, vi } from "vitest";

import type { CreatedIssue, IssueOut, TeamOut } from "../../api/api.gen";
import type { WorkApi } from "../../work/api";
import { createLocalConnector } from "../collab/local-connector";
import { body, h, p } from "../content";
import { PageEditor } from "../page-editor";
import { createTestEditor, memoryChannels, resolveGateway, settle } from "../test-editor";

import { anchorOf, targetOf } from "./anchor";
import { proposedTitle } from "./file-issue";
import { createMemoryCommentsApi } from "./memory-comments";

const TEAM = { key: "ENG", name: "Engineering" } as TeamOut;

const notUsed = () => Promise.reject(new Error("not used"));

function fakeWork() {
  const fileIssue = vi.fn<WorkApi["fileIssue"]>((issue) =>
    Promise.resolve({
      issue: { key: "ENG-42", title: issue.title } as IssueOut,
      chip: { key: "ENG-42", doc: "page-1", block: "blk-aaaaaaaa" },
    } satisfies CreatedIssue),
  );
  return {
    ...createMemoryCommentsApi("priya", () => new Date("2026-10-29T20:00:00Z")),
    myWork: notUsed,
    addTask: notUsed,
    setDone: notUsed,
    dismissMention: notUsed,
    settings: notUsed,
    saveSettings: notUsed,
    teams: () => Promise.resolve([TEAM]),
    issues: notUsed,
    issue: notUsed,
    searchIssues: notUsed,
    createIssue: notUsed,
    fileIssue,
    setState: notUsed,
    setPriority: notUsed,
    comments: notUsed,
    addComment: notUsed,
    queryIssues: notUsed,
    inbox: notUsed,
    triage: notUsed,
    cycles: notUsed,
    myIssues: notUsed,
    views: notUsed,
    createView: notUsed,
    updateView: notUsed,
    deleteView: notUsed,
    teamSettings: notUsed,
    saveTeamSettings: notUsed,
    createTeam: notUsed,
    setTeamArchived: notUsed,
  };
}

/** Tiptap keeps its editor on the view's DOM node. */
function editorOf(): Editor {
  const element = screen.getByRole("textbox", { name: "Page body" });
  const editor = (element as HTMLElement & { editor?: Editor }).editor;
  if (!editor) throw new Error("no editor");
  return editor;
}

/** Selects `text` inside the document. */
function select(editor: Editor, text: string) {
  let range: { from: number; to: number } | undefined;
  editor.state.doc.descendants((node, pos) => {
    if (range === undefined && node.isText && node.text?.includes(text)) {
      const from = pos + node.text.indexOf(text);
      range = { from, to: from + text.length };
    }
  });
  if (!range) throw new Error(`no "${text}" in the page`);
  editor.view.dispatch(
    editor.state.tr.setSelection(TextSelection.create(editor.state.doc, range.from, range.to)),
  );
}

async function openEditor(work: WorkApi) {
  const { factory } = memoryChannels();
  render(
    <PageEditor
      pageId="page-1"
      spaceId="space-1"
      seed={body(h(1, "Plan"), p("Ship the beta in May."))}
      connector={createLocalConnector({ channel: factory })}
      user={{ name: "Priya", color: "#1a73e8" }}
      resolvePage={resolveGateway}
      onOpenPage={vi.fn<(id: string) => void>()}
      work={work}
      me="priya"
    />,
  );
  await settle();
  return editorOf();
}

describe("anchors", () => {
  it("reads the block ID and quote of a selection", () => {
    const id = "blk-aaaaaaaa";
    const editor = createTestEditor({
      content: {
        type: "doc",
        content: [
          {
            type: "paragraph",
            attrs: { blockId: id },
            content: [{ type: "text", text: "Ship the beta in May." }],
          },
        ],
      },
    });
    select(editor, "the beta");
    expect(anchorOf(editor.state)).toEqual({ blockId: id, quote: "the beta" });
    expect(targetOf("doc-1", id)).toBe(`doc-1#${id}`);
    expect(targetOf("doc-1")).toBe("doc-1");
  });

  it("proposes a short title from a long selection", () => {
    expect(proposedTitle("Fix the flaky sign-in test\nmore detail")).toBe(
      "Fix the flaky sign-in test",
    );
    const long = "word ".repeat(40);
    expect(proposedTitle(long).length).toBeLessThanOrEqual(81);
    expect(proposedTitle(long).endsWith("…")).toBe(true);
  });
});

describe("comments panel", () => {
  it("is absent without a server", async () => {
    const { factory } = memoryChannels();
    render(
      <PageEditor
        pageId="page-1"
        spaceId="space-1"
        seed={body(p("x"))}
        connector={createLocalConnector({ channel: factory })}
        user={{ name: "Priya", color: "#1a73e8" }}
        resolvePage={resolveGateway}
        onOpenPage={vi.fn<(id: string) => void>()}
      />,
    );
    await settle();
    expect(screen.queryByRole("button", { name: "Comments" })).toBeNull();
  });

  it("starts a thread on the selected block, marks it, and resolves it", async () => {
    const user = userEvent.setup();
    const work = fakeWork();
    const editor = await openEditor(work);
    select(editor, "the beta");
    expect(anchorOf(editor.state).blockId).toBeUndefined();
    await user.click(screen.getByRole("button", { name: "Comments" }));
    // A block without an ID yet gets one, so the thread can point at it.
    const blockId = anchorOf(editor.state).blockId;
    expect(blockId).toMatch(/^blk-/);
    const panel = screen.getByRole("complementary", { name: "Comments" });
    expect(within(panel).getByText("New thread on “the beta”")).toBeDefined();
    expect(within(panel).getByText("No open threads on this page.")).toBeDefined();

    await user.type(within(panel).getByRole("textbox", { name: "Comment" }), "Is May right?");
    await user.click(within(panel).getByRole("button", { name: "Comment" }));
    await within(panel).findByText("Is May right?");
    expect(work.all()[0]?.target).toEqual({ doc: "page-1", block: blockId });
    expect(work.all()[0]?.quote).toBe("the beta");
    await waitFor(() => {
      expect(editor.view.dom.querySelectorAll(".vl-commented")).toHaveLength(1);
    });

    await user.type(within(panel).getByRole("textbox", { name: "Reply" }), "Yes");
    await user.click(within(panel).getByRole("button", { name: "Reply" }));
    await within(panel).findByText("Yes");

    const [like] = within(panel).getAllByRole("button", { name: "👍 (0)" });
    if (!like) throw new Error("no like button");
    await user.click(like);
    const liked = await within(panel).findByRole("button", { name: "👍 (1)" });
    expect(liked.getAttribute("aria-pressed")).toBe("true");

    await user.type(within(panel).getByRole("textbox", { name: "Assign this thread to" }), "@sam");
    await user.click(within(panel).getByRole("button", { name: "Assign" }));
    await within(panel).findByText("Assigned to @sam");

    await user.click(within(panel).getByRole("button", { name: "Resolve" }));
    await within(panel).findByText("No open threads on this page.");
    await waitFor(() => {
      expect(editor.view.dom.querySelectorAll(".vl-commented")).toHaveLength(0);
    });
    await user.click(within(panel).getByRole("button", { name: "Resolved" }));
    expect(within(panel).getByText("Is May right?")).toBeDefined();
    await user.click(within(panel).getByRole("button", { name: "Reopen" }));
    await user.click(within(panel).getByRole("button", { name: "Open" }));
    expect(within(panel).getByText("Is May right?")).toBeDefined();

    await user.click(within(panel).getByRole("button", { name: "Close comments" }));
    expect(screen.queryByRole("complementary", { name: "Comments" })).toBeNull();
  });

  it("files an issue from the selection and writes its chip into the block", async () => {
    const user = userEvent.setup();
    const work = fakeWork();
    const editor = await openEditor(work);
    select(editor, "Ship the beta");
    await user.click(screen.getByRole("button", { name: "Create issue from selection" }));
    const blockId = anchorOf(editor.state).blockId;
    const dialog = await screen.findByRole("dialog", { name: "Create issue" });
    const title = await within(dialog).findByRole("textbox", { name: "Title" });
    expect((title as HTMLInputElement).value).toBe("Ship the beta");
    await user.click(within(dialog).getByRole("button", { name: "Create issue" }));
    await waitFor(() => {
      expect(work.fileIssue).toHaveBeenCalledWith({
        team: "ENG",
        title: "Ship the beta",
        sourceBlock: `page-1#${String(blockId)}`,
        quote: "Ship the beta",
      });
    });
    await waitFor(() => {
      expect(editor.view.dom.querySelector(".vl-issue")?.textContent).toBe("ENG-42");
    });
    expect(editor.state.doc.child(1).textContent).toContain("Ship the beta in May.");
    expect(screen.queryByRole("dialog", { name: "Create issue" })).toBeNull();
  });
});
