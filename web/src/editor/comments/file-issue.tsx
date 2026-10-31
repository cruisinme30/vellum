// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor } from "@tiptap/core";
import { useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";
import { Popover } from "../../shell/popover";
import type { WorkApi } from "../../work/api";

import { blockPosition, targetOf, type Anchor } from "./anchor";
import { field } from "./thread";

/** The longest title proposed from a selection. */
const TITLE_LIMIT = 80;

/** A title proposed from the selected text: its first line, cut at a word. */
export function proposedTitle(quote: string): string {
  const line = quote.split("\n")[0]?.trim() ?? "";
  if (line.length <= TITLE_LIMIT) return line;
  const cut = line.slice(0, TITLE_LIMIT);
  const space = cut.lastIndexOf(" ");
  return `${(space > 40 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/**
 * Writes an issue chip at the end of the block the issue was filed from,
 * or after the selection when that block is gone.
 */
function insertIssueChip(
  editor: Editor,
  key: string,
  blockId: string | undefined,
) {
  const chip = [
    { type: "text", text: " " },
    { type: "issueKey", attrs: { key } },
  ];
  const pos =
    blockId === undefined
      ? undefined
      : blockPosition(editor.state.doc, blockId);
  const block = pos === undefined ? undefined : editor.state.doc.nodeAt(pos);
  const end =
    block?.isTextblock === true && pos !== undefined
      ? pos + block.nodeSize - 1
      : editor.state.selection.to;
  editor.chain().insertContentAt(end, chip).run();
}

/** Props of {@link FileIssue}. */
export interface FileIssueProperties {
  readonly api: WorkApi;
  readonly documentId: string;
  readonly editor: Editor;
  readonly anchor: Anchor;
  readonly onClose: () => void;
  /** Told the new issue's key once its chip is written. */
  readonly onFiled?: (key: string) => void;
}

/**
 * "Create issue" from a page selection (R-WK-7): picks a team, proposes a
 * title from the selection, files the issue with a link back to the block
 * and writes the issue's chip into the page.
 */
export function FileIssue({
  api,
  documentId,
  editor,
  anchor,
  onClose,
  onFiled,
}: FileIssueProperties) {
  const t = useT();
  const teams = useLoaded(() => api.teams(), [api], failureText);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const team = field(event.currentTarget, "team");
    const title = field(event.currentTarget, "title");
    if (team === "" || title === "") return;
    setBusy(true);
    setError(undefined);
    api
      .fileIssue({
        team,
        title,
        ...(anchor.blockId === undefined
          ? {}
          : { sourceBlock: targetOf(documentId, anchor.blockId) }),
        ...(anchor.quote === "" ? {} : { quote: anchor.quote }),
      })
      .then(
        (created) => {
          const key = created.chip?.key ?? created.issue.key;
          if (editor.isEditable) insertIssueChip(editor, key, anchor.blockId);
          onFiled?.(key);
          onClose();
        },
        (error_: unknown) => {
          setBusy(false);
          setError(failureText(error_));
        },
      );
  };
  return (
    <Popover
      label={t("file-issue-title")}
      onClose={onClose}
      className="vl-file-issue-popover"
    >
      <form className="vl-file-issue" onSubmit={submit}>
        <h2>{t("file-issue-title")}</h2>
        {teams.state === "loading" && (
          <p className="vl-muted" role="status">
            {t("loading")}
          </p>
        )}
        {teams.state === "failed" && (
          <p className="vl-field-error" role="alert">
            {teams.message}
            <CopyDiagnostic message={teams.message} />
          </p>
        )}
        {teams.state === "ready" && teams.value.length === 0 && (
          <p className="vl-muted">{t("file-issue-no-teams")}</p>
        )}
        {teams.state === "ready" && teams.value.length > 0 && (
          <>
            <label>
              {t("file-issue-team")}
              <select name="team" defaultValue={teams.value[0]?.key}>
                {teams.value.map((team) => (
                  <option key={team.key} value={team.key}>
                    {team.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {t("file-issue-name")}
              <input
                name="title"
                required
                defaultValue={proposedTitle(anchor.quote)}
              />
            </label>
            <button type="submit" className="vl-filled-button" disabled={busy}>
              {t("file-issue-create")}
            </button>
          </>
        )}
        {error !== undefined && (
          <p className="vl-field-error" role="alert">
            {error}
            <CopyDiagnostic message={error} />
          </p>
        )}
      </form>
    </Popover>
  );
}
