// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { NavLink, useNavigate } from "react-router";

import { useServices, useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { DESTINATIONS } from "./destinations";
import { Icon } from "./icon";
import { PageTree } from "./page-tree";
import { ThemeSettings } from "./theme-menu";

interface SidebarProperties {
  readonly currentId: string | undefined;
  /** On phones the sidebar is the "More" sheet and also lists the other destinations. */
  readonly compact: boolean;
  readonly onNavigate: () => void;
}

/** The sidebar: one page tree per space (brief §13). */
export function Sidebar({ currentId, compact, onNavigate }: SidebarProperties) {
  const t = useT();
  const navigate = useNavigate();
  const { store } = useServices();
  const workspace = useWorkspace();
  return (
    <div className="vl-sidebar-inner">
      <p className="vl-workspace-name">{workspace.name}</p>
      {compact && (
        <nav aria-label={t("nav-more-label")} className="vl-more-links">
          {[DESTINATIONS.favorites, DESTINATIONS.data, DESTINATIONS.help, DESTINATIONS.admin].map(
            (d) => (
              <NavLink key={d.id} to={d.path} className="vl-more-link" onClick={onNavigate}>
                <Icon name={d.icon} />
                {t(`nav-${d.id}`)}
              </NavLink>
            ),
          )}
        </nav>
      )}
      {workspace.spaces.map((space) => (
        <section key={space.id} className="vl-space" aria-labelledby={`space-${space.id}`}>
          <div className="vl-space-head">
            <h2 id={`space-${space.id}`}>
              <span aria-hidden="true">{space.icon}</span> {space.name}
            </h2>
            <button
              type="button"
              className="vl-icon-button"
              aria-label={t("space-add-page", { space: space.name })}
              onClick={() => {
                const page = store.createPage({ spaceId: space.id, parentId: undefined });
                void navigate(`/p/${page.id}`);
                onNavigate();
              }}
            >
              <Icon name="add" size={20} />
            </button>
          </div>
          <PageTree
            space={space}
            pages={workspace.pages}
            currentId={currentId}
            onNavigate={onNavigate}
          />
        </section>
      ))}
      {compact && (
        <section className="vl-space" aria-label={t("theme-title")}>
          <ThemeSettings />
        </section>
      )}
    </div>
  );
}
