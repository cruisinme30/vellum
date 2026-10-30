// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { failureText } from "../data/failure";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";

import type { DriveApi, NodeView, SpaceView } from "./api";
import { DetailsPanel } from "./details-panel";
import { formatBytes, formatWhen } from "./format";
import { openPath } from "./paths";
import { PlaceDialog, type PlaceMode } from "./place-dialog";

/** What the table tells its screen after a change, so it can reload. */
export interface TableEvents {
  /** An item moved, was copied or trashed, or a shortcut was made. */
  readonly onChanged: () => void;
}

const KIND_MARK: Record<NodeView["kind"], string> = {
  folder: "📁",
  page: "📄",
  book: "📕",
  collection: "🗂️",
  shortcut: "↪️",
};

/**
 * An item's name: a link to where it opens, or for a shortcut a button that
 * follows it.
 */
function ItemName({
  api,
  node,
}: {
  readonly api: DriveApi;
  readonly node: NodeView;
}) {
  const t = useT();
  const navigate = useNavigate();
  const [problem, setProblem] = useState<string>();
  const path = openPath(node);
  const mark = (
    <span aria-hidden="true" className="vl-drive-kind">
      {KIND_MARK[node.kind]}
    </span>
  );
  if (path !== undefined) {
    return (
      <>
        {mark}
        <Link to={path}>{node.title}</Link>
        <span className="vl-visually-hidden">
          {", " + t(`drive-kind-${node.kind}`)}
        </span>
      </>
    );
  }
  const follow = async () => {
    try {
      const target = await api.target(node.id);
      const to = openPath(target);
      if (to !== undefined) await navigate(to);
    } catch (error) {
      setProblem(failureText(error));
    }
  };
  return (
    <>
      {mark}
      <button
        type="button"
        className="vl-link-button"
        onClick={() => void follow()}
      >
        {node.title}
      </button>
      <span className="vl-visually-hidden">{`, ${t("drive-kind-shortcut")}`}</span>
      {problem !== undefined && (
        <small className="vl-field-error" role="alert">
          {problem}
          <CopyDiagnostic message={problem} />
        </small>
      )}
    </>
  );
}

function RowActions({
  node,
  starred,
  onStar,
  onDetails,
  onPlace,
  onTrash,
}: {
  readonly node: NodeView;
  readonly starred: boolean;
  readonly onStar: () => void;
  readonly onDetails: () => void;
  readonly onPlace: (mode: PlaceMode) => void;
  readonly onTrash: () => void;
}) {
  const t = useT();
  const name = { title: node.title };
  return (
    <div className="vl-drive-actions">
      <button
        type="button"
        className="vl-text-button"
        aria-pressed={starred}
        aria-label={t(
          starred ? "drive-unstar-label" : "drive-star-label",
          name,
        )}
        onClick={onStar}
      >
        {starred ? "★" : "☆"}
      </button>
      <button
        type="button"
        className="vl-text-button"
        aria-label={t("drive-details-label", name)}
        onClick={onDetails}
      >
        {t("drive-details")}
      </button>
      {(["move", "copy", "shortcut"] as const).map((mode) => (
        <button
          key={mode}
          type="button"
          className="vl-text-button"
          aria-label={t(`drive-${mode}-label`, name)}
          onClick={() => {
            onPlace(mode);
          }}
        >
          {t(`drive-${mode}`)}
        </button>
      ))}
      <button
        type="button"
        className="vl-text-button"
        aria-label={t("drive-trash-label", name)}
        onClick={onTrash}
      >
        {t("drive-trash")}
      </button>
    </div>
  );
}

/**
 * Live items with their actions: star, details (activity and backlinks),
 * move, copy, shortcut and trash. A failed action shows its message; a
 * finished one tells the screen to reload.
 */
export function ItemTable({
  api,
  items,
  spaces,
  starred: initialStarred,
  empty,
  onChanged,
}: {
  readonly api: DriveApi;
  readonly items: readonly NodeView[];
  readonly spaces: readonly SpaceView[];
  /** IDs of the items the person starred. */
  readonly starred: ReadonlySet<string>;
  readonly empty: string;
} & TableEvents) {
  const t = useT();
  const [details, setDetails] = useState<NodeView>();
  const [placing, setPlacing] = useState<{ item: NodeView; mode: PlaceMode }>();
  const [starred, setStarred] = useState(initialStarred);
  const [notice, setNotice] = useState<string>();
  const [problem, setProblem] = useState<string>();

  const run = async (action: () => Promise<string>) => {
    setProblem(undefined);
    try {
      setNotice(await action());
    } catch (error) {
      setProblem(failureText(error));
    }
  };
  const star = (node: NodeView) =>
    run(async () => {
      const next = !starred.has(node.id);
      await api.star(node.id, next);
      const changed = new Set(starred);
      if (next) changed.add(node.id);
      else changed.delete(node.id);
      setStarred(changed);
      return t(next ? "drive-starred-done" : "drive-unstarred-done", {
        title: node.title,
      });
    });
  const trash = (node: NodeView) =>
    run(async () => {
      await api.trash(node.id);
      if (details?.id === node.id) setDetails(undefined);
      onChanged();
      return t("drive-trashed", { title: node.title });
    });

  return (
    <div className="vl-drive-body">
      <div className="vl-drive-main">
        <p role="status" className="vl-drive-notice">
          {notice}
        </p>
        {problem !== undefined && (
          <p className="vl-field-error" role="alert">
            {problem}
            <CopyDiagnostic message={problem} />
          </p>
        )}
        {items.length === 0 ? (
          <p className="vl-muted">{empty}</p>
        ) : (
          <table className="vl-drive-table">
            <thead>
              <tr>
                <th scope="col">{t("drive-name")}</th>
                <th scope="col">{t("drive-modified")}</th>
                <th scope="col">{t("drive-size")}</th>
                <th scope="col">
                  <span className="vl-visually-hidden">
                    {t("drive-actions")}
                  </span>
                </th>
              </tr>
            </thead>
            <tbody>
              {items.map((node) => (
                <tr key={node.id} aria-selected={details?.id === node.id}>
                  <td className="vl-drive-name">
                    <ItemName api={api} node={node} />
                  </td>
                  <td>{formatWhen(node.modified)}</td>
                  <td>
                    {node.kind === "folder" ? "—" : formatBytes(node.size)}
                  </td>
                  <td>
                    <RowActions
                      node={node}
                      starred={starred.has(node.id)}
                      onStar={() => void star(node)}
                      onDetails={() => {
                        setDetails(node);
                      }}
                      onPlace={(mode) => {
                        setPlacing({ item: node, mode });
                      }}
                      onTrash={() => void trash(node)}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      {details !== undefined && (
        <DetailsPanel
          key={details.id}
          api={api}
          item={details}
          onClose={() => {
            setDetails(undefined);
          }}
        />
      )}
      {placing !== undefined && (
        <PlaceDialog
          api={api}
          item={placing.item}
          mode={placing.mode}
          spaces={spaces}
          onClose={() => {
            setPlacing(undefined);
          }}
          onDone={(result) => {
            const title = placing.item.title;
            setNotice(
              t(`drive-${placing.mode}-done`, { title, where: result.path }),
            );
            setPlacing(undefined);
            onChanged();
          }}
        />
      )}
    </div>
  );
}
