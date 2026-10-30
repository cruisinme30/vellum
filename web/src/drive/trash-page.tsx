// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link, useParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useT } from "../i18n/i18n";
import { CopyDiagnostic } from "../shell/copy-diagnostic";

import type { DriveApi, NodeView, SpaceView } from "./api";
import { DriveFrame } from "./drive-page";
import { formatBytes, formatWhen } from "./format";
import { loadSpaces, useReloadable } from "./loading";
import { spacePath } from "./paths";

/** A button that asks once more before an action that cannot be undone. */
function ConfirmButton({
  label,
  confirm,
  onConfirm,
}: {
  readonly label: string;
  readonly confirm: string;
  readonly onConfirm: () => void;
}) {
  const t = useT();
  const [asking, setAsking] = useState(false);
  if (!asking) {
    return (
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setAsking(true);
        }}
      >
        {label}
      </button>
    );
  }
  return (
    <span className="vl-drive-confirm">
      <button
        type="button"
        className="vl-filled-button vl-danger"
        onClick={onConfirm}
      >
        {confirm}
      </button>
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setAsking(false);
        }}
      >
        {t("drive-cancel")}
      </button>
    </span>
  );
}

/**
 * When an item trashed at `trashedAt` is purged, given the space's trash days.
 */
export function purgeDate(trashedAt: string, days: number): string | undefined {
  const at = new Date(trashedAt);
  if (Number.isNaN(at.getTime())) return undefined;
  return new Date(at.getTime() + days * 86_400_000).toISOString();
}

function TrashTable({
  api,
  space,
  items,
  onChanged,
}: {
  readonly api: DriveApi;
  readonly space: SpaceView;
  readonly items: readonly NodeView[];
  readonly onChanged: () => void;
}) {
  const t = useT();
  const [notice, setNotice] = useState<string>();
  const [problem, setProblem] = useState<string>();
  const run = async (action: () => Promise<string>) => {
    setProblem(undefined);
    try {
      setNotice(await action());
      onChanged();
    } catch (error) {
      setProblem(failureText(error));
    }
  };
  return (
    <>
      <p className="vl-muted">
        {t("drive-trash-retention", { days: space.settings.trash_days })}
      </p>
      <p role="status" className="vl-drive-notice">
        {notice}
      </p>
      {problem !== undefined && (
        <p className="vl-field-error" role="alert">
          {problem}
          <CopyDiagnostic message={problem} />
        </p>
      )}
      {items.length > 0 && (
        <ConfirmButton
          label={t("drive-empty-trash")}
          confirm={t("drive-empty-trash-confirm", { count: items.length })}
          onConfirm={() =>
            void run(async () =>
              t("drive-emptied", { count: await api.emptyTrash(space.id) }),
            )
          }
        />
      )}
      {items.length === 0 ? (
        <p className="vl-muted">{t("drive-trash-empty")}</p>
      ) : (
        <table className="vl-drive-table">
          <thead>
            <tr>
              <th scope="col">{t("drive-name")}</th>
              <th scope="col">{t("drive-trashed-at")}</th>
              <th scope="col">{t("drive-purged-at")}</th>
              <th scope="col">{t("drive-size")}</th>
              <th scope="col">
                <span className="vl-visually-hidden">{t("drive-actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {items.map((node) => {
              const trashedAt = node.trashed_at ?? undefined;
              const purgeAt =
                trashedAt === undefined
                  ? undefined
                  : purgeDate(trashedAt, space.settings.trash_days);
              return (
                <tr key={node.id}>
                  <td className="vl-drive-name">{node.title}</td>
                  <td>
                    {trashedAt === undefined ? "—" : formatWhen(trashedAt)}
                  </td>
                  <td>{purgeAt === undefined ? "—" : formatWhen(purgeAt)}</td>
                  <td>{formatBytes(node.size)}</td>
                  <td className="vl-drive-actions">
                    <button
                      type="button"
                      className="vl-text-button"
                      aria-label={t("drive-restore-label", {
                        title: node.title,
                      })}
                      onClick={() =>
                        void run(async () => {
                          await api.restore(node.id);
                          return t("drive-restored", { title: node.title });
                        })
                      }
                    >
                      {t("drive-restore")}
                    </button>
                    <ConfirmButton
                      label={t("drive-purge")}
                      confirm={t("drive-purge-confirm", { title: node.title })}
                      onConfirm={() =>
                        void run(async () => {
                          await api.purge(node.id);
                          return t("drive-purged", { title: node.title });
                        })
                      }
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </>
  );
}

function SpaceTrash({
  api,
  spaceId,
}: {
  readonly api: DriveApi;
  readonly spaceId: string;
}) {
  const t = useT();
  const { loaded, reload } = useReloadable(async () => {
    const [spaces, items] = await Promise.all([
      loadSpaces(api),
      api.trashList(spaceId),
    ]);
    return { space: spaces.find((space) => space.id === spaceId), items };
  }, [api, spaceId]);
  return (
    <Ready loaded={loaded}>
      {({ space, items }) =>
        space === undefined ? (
          <p className="vl-muted">{t("drive-space-missing")}</p>
        ) : (
          <>
            <h2>
              <Link to={spacePath(space.id)}>{space.name}</Link> ·{" "}
              {t("drive-trash-open")}
            </h2>
            <TrashTable
              api={api}
              space={space}
              items={items}
              onChanged={reload}
            />
          </>
        )
      }
    </Ready>
  );
}

/** A space's trash: restore, delete for good, or empty it (R-DR-4). */
export function TrashPage() {
  const { spaceId = "" } = useParams();
  return (
    <DriveFrame>
      {(api) => <SpaceTrash api={api} spaceId={spaceId} />}
    </DriveFrame>
  );
}
