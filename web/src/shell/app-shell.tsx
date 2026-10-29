// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Outlet, useLocation, useMatch, useNavigate } from "react-router";

import { useServiceHealth } from "../api/health";
import type { StoreProblem } from "../data/store";
import { useServices, useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { BottomBar, NavigationRail } from "./navigation";
import { Sidebar } from "./sidebar";
import { COMPACT, MEDIUM, useMedia } from "./use-media";

/**
 * The Material 3 app shell: navigation rail, sidebar page tree and main
 * area; on phones a bottom bar and a "More" sheet replace the rail and
 * sidebar.
 */
export function AppShell() {
  const t = useT();
  const compact = useMedia(COMPACT);
  const medium = useMedia(MEDIUM);
  const navigate = useNavigate();
  const location = useLocation();
  const { store } = useServices();
  const workspace = useWorkspace();
  const health = useServiceHealth();
  const currentId = useMatch("/p/:pageId")?.params.pageId;
  // The sidebar choice applies to the window size it was made in; the
  // "More" sheet belongs to the route it was opened on, so navigating
  // closes it without an effect.
  const [sidebarChoice, setSidebarChoice] = useState<{ medium: boolean; open: boolean }>();
  const [sheetPath, setSheetPath] = useState<string>();
  const sidebar = sidebarChoice?.medium === medium ? sidebarChoice.open : !medium;
  const sheet = sheetPath === location.pathname;
  const setSidebar = (open: boolean) => {
    setSidebarChoice({ medium, open });
  };
  const setSheet = (open: boolean) => {
    setSheetPath(open ? location.pathname : undefined);
  };

  const newPage = () => {
    const current = workspace.pages.find((p) => p.id === currentId);
    const spaceId = current?.spaceId ?? workspace.spaces[0]?.id;
    if (spaceId !== undefined) {
      void store.createPage({ spaceId, parentId: undefined }).then((page) => {
        if (page !== undefined) void navigate(`/p/${page.id}`);
      });
    }
  };
  const closeOverlay = () => {
    if (compact) {
      setSheet(false);
    } else if (medium) {
      setSidebar(false);
    }
  };
  const showSidebar = compact ? sheet : sidebar;

  return (
    <div className="vl-shell" data-compact={compact} data-sidebar={showSidebar}>
      <a className="vl-skip" href="#vl-main">
        {t("skip-to-content")}
      </a>
      {!compact && (
        <NavigationRail
          onNew={newPage}
          sidebarOpen={sidebar}
          onToggleSidebar={() => {
            setSidebar(!sidebar);
          }}
        />
      )}
      <aside
        id="vl-sidebar"
        className="vl-sidebar"
        aria-label={t("sidebar-label")}
        hidden={!showSidebar}
      >
        <Sidebar currentId={currentId} compact={compact} onNavigate={closeOverlay} />
      </aside>
      {showSidebar && (compact || medium) && (
        <div className="vl-scrim" aria-hidden="true" onClick={closeOverlay} />
      )}
      <main id="vl-main" className="vl-main" tabIndex={-1}>
        {health.degraded && <ServiceBanner />}
        {workspace.problem !== undefined && <WorkspaceProblem problem={workspace.problem} />}
        <Outlet />
      </main>
      {compact && (
        <BottomBar
          onNew={newPage}
          moreOpen={sheet}
          onMore={() => {
            setSheet(!sheet);
          }}
        />
      )}
    </div>
  );
}

/**
 * Shown while the server answers 503: part of Vellum (usually file storage)
 * is down, so pages still read but saving may fail (issue #191). It goes
 * away by itself on the next good answer.
 */
export function ServiceBanner() {
  const t = useT();
  return (
    <div className="vl-problem" role="status" data-testid="service-banner">
      <p>{t("service-degraded")}</p>
    </div>
  );
}

/** A change or load the server refused, with its request ID, until dismissed. */
function WorkspaceProblem({ problem }: { readonly problem: StoreProblem }) {
  const t = useT();
  const { store } = useServices();
  return (
    <div className="vl-problem" role="alert">
      <p>
        {problem.requestId === undefined
          ? problem.message
          : t("workspace-problem-request", {
              message: problem.message,
              request: problem.requestId,
            })}
      </p>
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          store.dismissProblem();
        }}
      >
        {t("workspace-problem-dismiss")}
      </button>
    </div>
  );
}
