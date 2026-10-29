// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import { useSession } from "../api/session";
import { useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { PageList } from "./page-list";

/** Home: recently changed pages and favourites. */
export function HomeView() {
  const t = useT();
  const workspace = useWorkspace();
  // Against a server the signed-in user greets; the demo has its own.
  const user = useSession()?.status.user;
  const recent = workspace.pages
    .toSorted((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    .slice(0, 6);
  const starred = workspace.pages.filter((p) => p.starred);
  return (
    <div className="vl-view">
      <h1>{t("home-greeting", { name: (user?.name ?? workspace.me.name).replace(/ .*/s, "") })}</h1>
      <nav className="vl-home-shortcuts" aria-label={t("home-shortcuts")}>
        <Link to="/drive">{t("drive")}</Link>
        <Link to="/my-work">{t("my-work")}</Link>
        <Link to="/issues">{t("issues")}</Link>
        <Link to="/books">{t("books")}</Link>
      </nav>
      <section aria-labelledby="home-recent">
        <h2 id="home-recent">{t("home-recent")}</h2>
        <PageList items={recent.map((page) => ({ page }))} empty={t("home-recent-empty")} />
      </section>
      <section aria-labelledby="home-starred">
        <h2 id="home-starred">{t("nav-favorites")}</h2>
        <PageList items={starred.map((page) => ({ page }))} empty={t("favorites-empty")} />
      </section>
    </div>
  );
}

/** Favorites: starred pages. */
export function FavoritesView() {
  const t = useT();
  const workspace = useWorkspace();
  return (
    <div className="vl-view">
      <h1>{t("nav-favorites")}</h1>
      <PageList
        items={workspace.pages.filter((p) => p.starred).map((page) => ({ page }))}
        empty={t("favorites-empty")}
      />
    </div>
  );
}
