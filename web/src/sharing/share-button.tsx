// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useServerApi } from "../data/server-apis";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { SharePanel } from "./share-panel";

/**
 * The Share button for a page or folder, and the dialog it opens (#95).
 * Both are absent in demo mode, which has nobody to share with.
 */
export function useShare(itemId: string) {
  const t = useT();
  const sharing = useServerApi("sharing");
  const directory = useServerApi("directory");
  const [openFor, setOpenFor] = useState<string>();
  const open = openFor === itemId;
  const button = sharing && (
    <button
      type="button"
      className="vl-icon-button"
      aria-label={t("share-open")}
      title={t("share-open")}
      aria-expanded={open}
      onClick={() => {
        setOpenFor(open ? undefined : itemId);
      }}
    >
      <Icon name="share" />
    </button>
  );
  const panel = sharing && open && (
    <SharePanel
      api={sharing}
      itemId={itemId}
      directory={directory}
      onClose={() => {
        setOpenFor(undefined);
      }}
    />
  );
  return { button, panel };
}
