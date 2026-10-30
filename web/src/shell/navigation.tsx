// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef, useState } from "react";
import { NavLink } from "react-router";

import { useT } from "../i18n/i18n";

import { DESTINATIONS, type Destination } from "./destinations";
import { Icon } from "./icon";
import { NewButton, type NewActions } from "./new-menu";
import { Popover } from "./popover";
import { ThemeSettings } from "./theme-menu";

function RailLink({ destination }: { readonly destination: Destination }) {
  const t = useT();
  return (
    <NavLink to={destination.path} end={destination.path === "/"} className="vl-rail-item">
      <span className="vl-rail-indicator">
        <Icon name={destination.icon} />
      </span>
      <span className="vl-rail-label">{t(`nav-${destination.id}`)}</span>
    </NavLink>
  );
}

interface NavProperties {
  /** What New makes: a page, and a book where books can be made. */
  readonly onNew: NewActions;
}

/** The Material 3 navigation rail, for windows 600 px and wider. */
export function NavigationRail({
  onNew,
  onToggleSidebar,
  sidebarOpen,
}: NavProperties & { readonly onToggleSidebar: () => void; readonly sidebarOpen: boolean }) {
  const t = useT();
  const [theme, setTheme] = useState(false);
  return (
    <nav className="vl-rail" aria-label={t("nav-label")}>
      <button
        type="button"
        className="vl-icon-button"
        aria-label={t(sidebarOpen ? "nav-hide-pages" : "nav-show-pages")}
        aria-expanded={sidebarOpen}
        aria-controls="vl-sidebar"
        onClick={onToggleSidebar}
      >
        <Icon name="menu" />
      </button>
      <NewButton actions={onNew} className="vl-fab">
        <Icon name="edit" />
        <span className="vl-rail-label">{t("nav-new")}</span>
      </NewButton>
      <RailLink destination={DESTINATIONS.home} />
      <RailLink destination={DESTINATIONS.search} />
      <RailLink destination={DESTINATIONS.favorites} />
      <RailLink destination={DESTINATIONS.inbox} />
      <RailLink destination={DESTINATIONS.data} />
      <span className="vl-rail-spacer" />
      <span className="vl-menu-anchor">
        <button
          type="button"
          className="vl-rail-item"
          aria-expanded={theme}
          aria-haspopup="dialog"
          onClick={() => {
            setTheme(!theme);
          }}
        >
          <span className="vl-rail-indicator">
            <Icon name="palette" />
          </span>
          <span className="vl-rail-label">{t("nav-theme")}</span>
        </button>
        {theme && (
          <Popover
            label={t("theme-title")}
            className="vl-popover-rail"
            onClose={() => {
              setTheme(false);
            }}
          >
            <ThemeSettings />
          </Popover>
        )}
      </span>
      <RailLink destination={DESTINATIONS.help} />
      <RailLink destination={DESTINATIONS.admin} />
    </nav>
  );
}

/** The bottom navigation bar, for phones (below 600 px). */
export function BottomBar({
  onNew,
  onMore,
  moreOpen,
}: NavProperties & { readonly onMore: () => void; readonly moreOpen: boolean }) {
  const t = useT();
  const bar = useRef<HTMLElement>(null);
  // Tab must not leave the focused control under the bar (WCAG 2.4.11).
  // The root's scroll padding covers browsers that scroll a focused control
  // fully into view, but Firefox scrolls only when none of it is in view,
  // so a tall row peeking above the bar stayed mostly hidden.
  useEffect(() => {
    const onFocus = (event: FocusEvent) => {
      const target = event.target;
      const element = bar.current;
      if (!(target instanceof Element) || element === null || element.contains(target)) return;
      if (!target.matches(":focus-visible")) return;
      if (target.getBoundingClientRect().bottom > element.getBoundingClientRect().top)
        target.scrollIntoView({ block: "nearest" });
    };
    document.addEventListener("focusin", onFocus);
    return () => {
      document.removeEventListener("focusin", onFocus);
    };
  }, []);
  return (
    // The bar floats over the page by design (Material 3); the page's bottom
    // padding lets the last content scroll clear of it. The UI suite's
    // overlap check is told so.
    <nav ref={bar} className="vl-bottom-bar" aria-label={t("nav-label")} data-allow-overlap="">
      <NewButton actions={onNew} className="vl-rail-item">
        <span className="vl-rail-indicator">
          <Icon name="edit" />
        </span>
        <span className="vl-rail-label">{t("nav-new")}</span>
      </NewButton>
      <RailLink destination={DESTINATIONS.home} />
      <RailLink destination={DESTINATIONS.search} />
      <RailLink destination={DESTINATIONS.inbox} />
      <button
        type="button"
        className="vl-rail-item"
        aria-expanded={moreOpen}
        aria-controls="vl-sidebar"
        onClick={onMore}
      >
        <span className="vl-rail-indicator">
          <Icon name="menu" />
        </span>
        <span className="vl-rail-label">{t("nav-more")}</span>
      </button>
    </nav>
  );
}
