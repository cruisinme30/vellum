// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Editor } from "@tiptap/core";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  CreatedIssue,
  IssueOut,
  TeamOut,
  ThreadOut,
} from "../../api/api.gen";
import { I18nProvider } from "../../i18n/i18n";
import type { WorkApi } from "../../work/api";
import { createTestEditor } from "../test-editor";

import { CommentsPanel } from "./comments-panel";
import { FileIssue, proposedTitle } from "./file-issue";
import { createMemoryCommentsApi } from "./memory-comments";

const BLOCK = "blk-aaaaaaaa";
const OTHER = "blk-bbbbbbbb";
const CLOCK = () => new Date("2026-10-29T20:00:00Z");

/** The thread a call answered with; the memory store always answers one. */
async function must(call: Promise<ThreadOut | undefined>): Promise<ThreadOut> {
  const thread = await call;
  if (thread === undefined) throw new Error("no thread");
  return thread;
}

let editors: Editor[] = [];
afterEach(() => {
  for (const editor of editors) editor.destroy();
  editors = [];
});

function editorWith(
  html = `<p data-block-id="${BLOCK}">Ship the beta</p><p>Tail</p>`,
) {
  const editor = createTestEditor({ content: html });
  editors.push(editor);
  return editor;
}

describe("comments in memory", () => {
  it(
    "lists a page's threads with its block threads, or one " + "block's",
    async () => {
      const api = createMemoryCommentsApi("priya", CLOCK);
      await api.startThread({ target: "page-1", body: "On the page" });
      await api.startThread({
        target: `page-1#${BLOCK}`,
        body: "On a block",
        quote: "beta",
      });
      await api.startThread({
        target: `page-1#${OTHER}`,
        body: "Elsewhere",
        assignTo: "sam",
      });
      await api.startThread({ target: "ENG-7", body: "On an issue" });
      expect(await api.threads("page-1")).toHaveLength(3);
      const one = await api.threads(`page-1#${BLOCK}`);
      expect(one.map((t) => t.quote)).toEqual(["beta"]);
      const issue = await api.threads("ENG-7");
      expect(issue[0]?.target).toEqual({ issue: "ENG-7" });
      expect(api.all().find((t) => t.target.block === OTHER)?.assignee).toBe(
        "sam",
      );
    },
  );

  it("replies, edits, deletes, reacts, resolves and assigns", async () => {
    const api = createMemoryCommentsApi("priya");
    const thread = await must(
      api.startThread({ target: "page-1", body: "First" }),
    );
    const first = thread.comments[0]?.id ?? "";
    const replied = await must(api.reply(first, "Second"));
    expect(replied.comments.map((c) => c.body)).toEqual(["First", "Second"]);
    const edited = await must(api.editComment(first, "First, edited"));
    expect(edited.comments[0]?.edited_at).toBeDefined();
    const deleted = await must(api.deleteComment(first));
    expect(deleted.comments[0]).toMatchObject({ deleted: true, body: "" });

    const second = replied.comments[1]?.id ?? "";
    await api.react(second, "👍", true);
    let reacted = await must(api.react(second, "🎉", true));
    expect(reacted.comments[1]?.reactions.map((r) => r.emoji)).toEqual([
      "👍",
      "🎉",
    ]);
    reacted = await must(api.react(second, "👍", false));
    expect(reacted.comments[1]?.reactions).toEqual([
      { emoji: "🎉", by: ["priya"] },
    ]);
    // The first comment is untouched by reactions on the second.
    expect(reacted.comments[0]?.reactions).toEqual([]);

    const resolved = await must(api.resolve(thread.id, true));
    expect(resolved).toMatchObject({ resolved_by: "priya" });
    const reopened = await must(api.resolve(thread.id, false));
    expect(reopened.resolved_at).toBeUndefined();
    const assigned = await must(api.assign(thread.id, "sam"));
    expect(assigned.assignee).toBe("sam");
    let nobody: string | undefined;
    const unassigned = await must(api.assign(thread.id, nobody));
    expect(unassigned.assignee).toBeUndefined();
  });

  it("refuses unknown comments and threads", async () => {
    const api = createMemoryCommentsApi("priya");
    await expect(api.reply("nope", "x")).rejects.toThrow(
      "unknown comment nope",
    );
    await expect(api.resolve("nope", true)).rejects.toThrow(
      "unknown thread nope",
    );
  });
});

function renderPanel(
  api: ReturnType<typeof createMemoryCommentsApi>,
  options: { quote?: string; blockId?: string; me?: string } = {},
) {
  const user = userEvent.setup();
  const editor = editorWith();
  const onClose = vi.fn();
  render(
    <I18nProvider>
      <CommentsPanel
        api={api}
        documentId="page-1"
        editor={editor}
        me={options.me}
        anchor={{ blockId: options.blockId, quote: options.quote ?? "" }}
        onClose={onClose}
      />
    </I18nProvider>,
  );
  return { user, editor, onClose };
}

describe("comments panel", () => {
  it("says when a load fails", async () => {
    const api = createMemoryCommentsApi("priya");
    vi.spyOn(api, "threads").mockRejectedValue(new Error("Server down."));
    renderPanel(api);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Server down.");
  });

  it(
    "starts a page thread with an assignee, then works the " + "thread",
    async () => {
      const api = createMemoryCommentsApi("priya", CLOCK);
      const start = vi.spyOn(api, "startThread");
      const { user, onClose } = renderPanel(api, { me: "priya" });
      expect(
        await screen.findByText("No open threads on this page."),
      ).toBeDefined();
      expect(screen.getByText("New thread on this page")).toBeDefined();

      await user.type(
        screen.getByRole("textbox", { name: "Comment" }),
        "Who owns this?",
      );
      await user.type(
        screen.getByRole("textbox", { name: "Assign to" }),
        "@sam",
      );
      await user.click(screen.getByRole("button", { name: "Comment" }));
      expect(start).toHaveBeenCalledWith({
        target: "page-1",
        body: "Who owns this?",
        assignTo: "sam",
      });
      expect(await screen.findByText("Assigned to @sam")).toBeDefined();
      // A page thread has no block to show.
      expect(screen.queryByRole("button", { name: "Show in page" })).toBeNull();

      await user.click(screen.getByRole("button", { name: "Unassign" }));
      expect(await screen.findByText("Not assigned")).toBeDefined();
      await user.type(
        screen.getByRole("textbox", { name: "Assign this thread to" }),
        "@ada",
      );
      await user.click(screen.getByRole("button", { name: "Assign" }));
      expect(await screen.findByText("Assigned to @ada")).toBeDefined();

      await user.type(screen.getByRole("textbox", { name: "Reply" }), "I do.");
      await user.click(screen.getByRole("button", { name: "Reply" }));
      expect(await screen.findByText("I do.")).toBeDefined();

      const reactions =
        screen.getAllByRole("group", { name: "Reactions" })[0] ?? document.body;
      await user.click(
        within(reactions).getByRole("button", { name: "👍 (0)" }),
      );
      const on = await screen.findByRole("button", { name: "👍 (1)" });
      expect(on.getAttribute("aria-pressed")).toBe("true");
      await user.click(on);
      expect(
        await screen.findAllByRole("button", { name: "👍 (0)" }),
      ).toHaveLength(2);

      await user.click(screen.getByRole("button", { name: "Resolve" }));
      expect(
        await screen.findByText("No open threads on this page."),
      ).toBeDefined();
      await user.click(screen.getByRole("button", { name: "Resolved" }));
      expect(await screen.findByText(/resolved by @priya/u)).toBeDefined();
      await user.click(screen.getByRole("button", { name: "Reopen" }));
      expect(await screen.findByText("No resolved threads yet.")).toBeDefined();

      await user.click(screen.getByRole("button", { name: "Close comments" }));
      expect(onClose).toHaveBeenCalled();
    },
  );

  it(
    "starts a thread on a quote, and shows its block in the " + "page",
    async () => {
      const api = createMemoryCommentsApi("priya", CLOCK);
      const { user, editor } = renderPanel(api, {
        blockId: BLOCK,
        quote: "beta",
      });
      expect(await screen.findByText("New thread on “beta”")).toBeDefined();
      await user.type(
        screen.getByRole("textbox", { name: "Comment" }),
        "Which beta?",
      );
      await user.click(screen.getByRole("button", { name: "Comment" }));
      expect(
        await screen.findByText("beta", { selector: "blockquote" }),
      ).toBeDefined();
      await user.click(screen.getByRole("button", { name: "Show in page" }));
      expect(editor.state.selection.$from.parent.attrs["blockId"]).toBe(BLOCK);
    },
  );

  it("names a block thread without a quote, and shows failures", async () => {
    const api = createMemoryCommentsApi("priya");
    vi.spyOn(api, "startThread").mockRejectedValue(new Error("Not allowed."));
    const { user } = renderPanel(api, { blockId: BLOCK });
    expect(
      await screen.findByText("New thread on the block at the cursor"),
    ).toBeDefined();
    await user.type(screen.getByRole("textbox", { name: "Comment" }), "Hi");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Not allowed.");
  });

  it("ignores a reveal for a block that is gone", async () => {
    const api = createMemoryCommentsApi("priya");
    await api.startThread({ target: `page-1#${OTHER}`, body: "Lost block" });
    const { user, editor } = renderPanel(api);
    const before = editor.state.selection.from;
    await user.click(
      await screen.findByRole("button", { name: "Show in page" }),
    );
    expect(editor.state.selection.from).toBe(before);
  });
});

const TEAM = { key: "ENG", name: "Engineering" } as TeamOut;

function workWith(overrides: Partial<WorkApi>): WorkApi {
  return {
    teams: () => Promise.resolve([TEAM]),
    fileIssue: (issue) =>
      Promise.resolve({
        issue: { key: "ENG-9", title: issue.title } as IssueOut,
      } as CreatedIssue),
    ...overrides,
  } as WorkApi;
}

function renderFileIssue(
  work: WorkApi,
  anchor: { blockId?: string; quote: string },
  editable = true,
) {
  const user = userEvent.setup();
  const editor = editorWith();
  editor.setEditable(editable);
  const onClose = vi.fn();
  const onFiled = vi.fn();
  render(
    <I18nProvider>
      <FileIssue
        api={work}
        documentId="page-1"
        editor={editor}
        anchor={{ blockId: anchor.blockId, quote: anchor.quote }}
        onClose={onClose}
        onFiled={onFiled}
      />
    </I18nProvider>,
  );
  return { user, editor, onClose, onFiled };
}

describe("file an issue", () => {
  it("cuts a long title at a word, or mid-word when there is none", () => {
    const words = "word ".repeat(30);
    expect(proposedTitle(words).endsWith("word…")).toBe(true);
    const long = "x".repeat(100);
    expect(proposedTitle(long)).toBe(`${"x".repeat(80)}…`);
    expect(proposedTitle("first\nsecond")).toBe("first");
  });

  it("says when teams fail to load, or there are none", async () => {
    renderFileIssue(
      workWith({ teams: () => Promise.reject(new Error("No teams API.")) }),
      {
        quote: "",
      },
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("No teams API.");
  });

  it("asks for a team first when there are none", async () => {
    renderFileIssue(workWith({ teams: () => Promise.resolve([]) }), {
      quote: "",
    });
    expect(
      await screen.findByText("Create a team in Work first."),
    ).toBeDefined();
  });

  it(
    "files from a caret, falling back to the issue key, without a chip when " +
      "read-only",
    async () => {
      const fileIssue = vi.fn<WorkApi["fileIssue"]>((issue) =>
        Promise.resolve({
          issue: { key: "ENG-9", title: issue.title } as IssueOut,
        } as CreatedIssue),
      );
      const { user, editor, onClose, onFiled } = renderFileIssue(
        workWith({ fileIssue }),
        { quote: "" },
        false,
      );
      await user.type(
        await screen.findByRole("textbox", { name: "Title" }),
        "Loose end",
      );
      await user.click(screen.getByRole("button", { name: "Create issue" }));
      await waitFor(() => {
        expect(onClose).toHaveBeenCalled();
      });
      expect(fileIssue).toHaveBeenCalledWith({
        team: "ENG",
        title: "Loose end",
      });
      expect(onFiled).toHaveBeenCalledWith("ENG-9");
      expect(editor.getHTML()).not.toContain("ENG-9");
    },
  );

  it("writes the chip after the selection when its block is gone", async () => {
    const { user, editor, onFiled } = renderFileIssue(workWith({}), {
      blockId: OTHER,
      quote: "Tail",
    });
    await user.click(
      await screen.findByRole("button", { name: "Create issue" }),
    );
    await waitFor(() => {
      expect(onFiled).toHaveBeenCalledWith("ENG-9");
    });
    expect(
      editor.getJSON().content.some((n) => JSON.stringify(n).includes("ENG-9")),
    ).toBe(true);
  });

  it("shows a failed create and lets you try again", async () => {
    const { user, onClose } = renderFileIssue(
      workWith({
        fileIssue: () => Promise.reject(new Error("Team is archived.")),
      }),
      { blockId: BLOCK, quote: "Ship the beta" },
    );
    await user.click(
      await screen.findByRole("button", { name: "Create issue" }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Team is archived.");
    expect(
      screen
        .getByRole("button", { name: "Create issue" })
        .hasAttribute("disabled"),
    ).toBe(false);
    expect(onClose).not.toHaveBeenCalled();
  });
});
