// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Link } from "react-router";

import { SLASH_ITEMS } from "../editor/slash/items";
import { useT } from "../i18n/i18n";

const SHORTCUTS: readonly [keys: string, id: string][] = [
  ["/", "help-key-slash"],
  ["Ctrl B · Ctrl I · Ctrl E", "help-key-format"],
  ["Ctrl Z · Ctrl Shift Z", "help-key-undo"],
  ["Tab · Shift Tab", "help-key-indent"],
  ["@name · [[Page]] · ENG-142 · 2026-11-24", "help-key-inline"],
  ["Alt Shift ↑ ↓ ← →", "help-key-tree"],
  ["Shift F10", "help-key-menu"],
  ["↑ ↓ ← → · Enter · F2 · Shift Enter", "help-key-grid"],
  ["Alt ← →", "help-key-board"],
];

/** Help: shortcuts and every block the slash menu inserts. */
export function HelpView() {
  const t = useT();
  return (
    <div className="vl-view">
      <h1>{t("nav-help")}</h1>
      <h2>{t("help-shortcuts")}</h2>
      <dl className="vl-shortcuts">
        {SHORTCUTS.map(([keys, id]) => (
          <div key={id}>
            <dt>
              <kbd>{keys}</kbd>
            </dt>
            <dd>{t(id)}</dd>
          </div>
        ))}
      </dl>
      <h2>{t("help-blocks")}</h2>
      <ul className="vl-block-list">
        {SLASH_ITEMS.map((item) => (
          <li key={item.id}>
            <strong>{item.title}</strong> — {item.description}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** A destination whose workstream has not landed yet. */
export function PendingView({ title, body }: { readonly title: string; readonly body: string }) {
  return (
    <div className="vl-view">
      <h1>{title}</h1>
      <p className="vl-muted">{body}</p>
    </div>
  );
}

/** Admin. */
export function AdminView() {
  const t = useT();
  return <PendingView title={t("nav-admin")} body={t("admin-pending")} />;
}

/** Unknown routes and missing pages. */
export function NotFoundView() {
  const t = useT();
  return (
    <div className="vl-view">
      <h1>{t("not-found-title")}</h1>
      <p>
        <Link to="/">{t("not-found-home")}</Link>
      </p>
    </div>
  );
}
