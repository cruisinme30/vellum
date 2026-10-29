// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState, type SyntheticEvent } from "react";

import { failureText } from "../data/failure";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { DriveApi, NodeView, Place, SpaceView } from "./api";

/** What the dialog does with the item. */
export type PlaceMode = "move" | "copy" | "shortcut";

/** A destination folder or page, with its depth for indenting. */
interface Destination {
  readonly node: NodeView;
  readonly depth: number;
}

/**
 * The folders and pages an item can go into, in tree order. Moving leaves
 * out the item itself and everything under it, since nothing can move into
 * itself.
 */
export function destinations(
  containers: readonly NodeView[],
  item: NodeView,
  mode: PlaceMode,
): Destination[] {
  const parents = new Map(containers.map((node) => [node.id, node.parent_id ?? undefined]));
  const depthOf = (node: NodeView) => {
    let depth = 0;
    let parent = node.parent_id ?? undefined;
    while (parent !== undefined && depth < 64) {
      depth += 1;
      parent = parents.get(parent);
    }
    return depth;
  };
  const inside = (node: NodeView) => {
    let current: string | undefined = node.id;
    for (let step = 0; current !== undefined && step < 64; step += 1) {
      if (current === item.id) return true;
      current = parents.get(current);
    }
    return false;
  };
  return containers
    .filter((node) => mode !== "move" || !inside(node))
    .map((node) => ({ node, depth: depthOf(node) }));
}

const TOP = "";

/** Moves or copies an item, or makes a shortcut to it, at `to`. */
function placeItem(api: DriveApi, mode: PlaceMode, id: string, to: Place): Promise<NodeView> {
  switch (mode) {
    case "move": {
      return api.move(id, to);
    }
    case "copy": {
      return api.copy(id, to);
    }
    case "shortcut": {
      return api.shortcut(id, to);
    }
  }
}

function FolderChoice({
  api,
  item,
  mode,
  spaceId,
  value,
  onChange,
}: {
  readonly api: DriveApi;
  readonly item: NodeView;
  readonly mode: PlaceMode;
  readonly spaceId: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
}) {
  const t = useT();
  const id = useId();
  const loaded = useLoaded(() => api.containers(spaceId), [api, spaceId], failureText);
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
      </p>
    );
  }
  const list = loaded.state === "ready" ? destinations(loaded.value.items, item, mode) : [];
  return (
    <>
      <label htmlFor={id}>{t("drive-place-folder")}</label>
      <select
        id={id}
        value={value}
        disabled={loaded.state !== "ready"}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        <option value={TOP}>{t("drive-place-top")}</option>
        {list.map(({ node, depth }) => (
          <option key={node.id} value={node.id}>
            {`${"  ".repeat(depth + 1)}${node.title}`}
          </option>
        ))}
      </select>
      {loaded.state === "ready" && loaded.value.truncated && (
        <p className="vl-muted">{t("drive-place-truncated")}</p>
      )}
    </>
  );
}

/**
 * Moves or copies an item, or puts a shortcut to it, into a chosen space
 * and folder (R-DR-3). Escape or Cancel closes it without a change.
 */
export function PlaceDialog({
  api,
  item,
  mode,
  spaces,
  onDone,
  onClose,
}: {
  readonly api: DriveApi;
  readonly item: NodeView;
  readonly mode: PlaceMode;
  /** The spaces to choose from: My Drive first, then the shared drives. */
  readonly spaces: readonly SpaceView[];
  /** Called with the moved item, the copy or the new shortcut. */
  readonly onDone: (result: NodeView) => void;
  readonly onClose: () => void;
}) {
  const t = useT();
  const headingId = useId();
  const spaceSelect = useId();
  const panel = useRef<HTMLFormElement>(null);
  const initialSpace = spaces.some((space) => space.id === item.space_id)
    ? item.space_id
    : (spaces[0]?.id ?? "");
  const [spaceId, setSpaceId] = useState(initialSpace);
  const [parentId, setParentId] = useState(TOP);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string>();

  // The latest onClose, so Escape works without re-running the focus effect.
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  }, [onClose]);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>("select, button")?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      opener?.focus();
    };
  }, []);

  const submit = async (event: SyntheticEvent) => {
    event.preventDefault();
    if (spaceId === "") return;
    setBusy(true);
    setProblem(undefined);
    const to = { spaceId, parentId: parentId === TOP ? undefined : parentId };
    try {
      onDone(await placeItem(api, mode, item.id, to));
    } catch (error) {
      setProblem(failureText(error));
      setBusy(false);
    }
  };

  return (
    <div className="vl-drive-scrim">
      <form
        ref={panel}
        className="vl-drive-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId}
        onSubmit={(event) => void submit(event)}
      >
        <h2 id={headingId}>{t(`drive-${mode}-title`, { title: item.title })}</h2>
        <label htmlFor={spaceSelect}>{t("drive-place-space")}</label>
        <select
          id={spaceSelect}
          value={spaceId}
          onChange={(event) => {
            setSpaceId(event.target.value);
            setParentId(TOP);
          }}
        >
          {spaces.map((space) => (
            <option key={space.id} value={space.id}>
              {space.name}
            </option>
          ))}
        </select>
        {spaceId !== "" && (
          <FolderChoice
            key={spaceId}
            api={api}
            item={item}
            mode={mode}
            spaceId={spaceId}
            value={parentId}
            onChange={setParentId}
          />
        )}
        {problem !== undefined && (
          <p className="vl-field-error" role="alert">
            {problem}
          </p>
        )}
        <div className="vl-drive-dialog-actions">
          <button type="button" className="vl-text-button" onClick={onClose}>
            {t("drive-cancel")}
          </button>
          <button type="submit" className="vl-filled-button" disabled={busy || spaceId === ""}>
            {t(`drive-${mode}`)}
          </button>
        </div>
      </form>
    </div>
  );
}
