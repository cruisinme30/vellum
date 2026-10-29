// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef } from "react";
import { useNavigate } from "react-router";

import { useServices, useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

/**
 * `/new`, the installed app's "New page" shortcut: creates a page in the
 * first space and opens it. The ref keeps React's development double
 * effect from creating two pages.
 */
export function NewPageView() {
  const t = useT();
  const navigate = useNavigate();
  const { store } = useServices();
  const spaceId = useWorkspace().spaces[0]?.id;
  const created = useRef(false);

  useEffect(() => {
    if (created.current || spaceId === undefined) {
      return;
    }
    created.current = true;
    const page = store.createPage({ spaceId, parentId: undefined });
    void navigate(`/p/${page.id}`, { replace: true });
  }, [navigate, spaceId, store]);

  return (
    <p className="vl-muted" role="status">
      {t("editor-loading")}
    </p>
  );
}
