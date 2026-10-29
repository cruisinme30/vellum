// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";
import { Link } from "react-router";

import { useSession } from "../api/session";
import { SLASH_ITEMS } from "../editor/slash/items";
import { useT } from "../i18n/i18n";

import "../admin/console/console.css";

import { describeError } from "./sign-in";

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

const ADMIN_SCREENS = [
  "users",
  "spaces",
  "usage",
  "audit",
  "settings",
  "backups",
  "webhooks",
  "requests",
  "routes",
] as const;

/** Admin: the console's screens, and the signed-in account. */
export function AdminView() {
  const t = useT();
  return (
    <>
      <div className="vl-view">
        <h1>{t("nav-admin")}</h1>
        <p className="vl-muted">{t("admin-intro")}</p>
        <ul className="vl-admin-links">
          {ADMIN_SCREENS.map((screen) => (
            <li key={screen}>
              <Link to={`/admin/${screen}`}>{t(`admin-${screen}`)}</Link>
              <span className="vl-muted">{t(`admin-${screen}-summary`)}</span>
            </li>
          ))}
        </ul>
      </div>
      <AccountSection />
    </>
  );
}

/** Who is signed in, and a way out. Shown only against a Vellum server. */
function AccountSection() {
  const t = useT();
  const session = useSession();
  const [error, setError] = useState<string | undefined>();
  const user = session?.status.user;
  if (session === undefined || user === undefined || user === null) return;
  const signOut = () => {
    setError(undefined);
    session.auth
      .signOut()
      .then(() => session.refresh())
      .catch((error_: unknown) => {
        setError(describeError(error_, t("auth-error")));
      });
  };
  return (
    <section className="vl-view" aria-labelledby="account-heading">
      <h2 id="account-heading">{t("auth-signed-in-as", { name: user.name, email: user.email })}</h2>
      {error === undefined ? undefined : (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <button type="button" className="vl-filled-button" onClick={signOut}>
        {t("auth-sign-out")}
      </button>
    </section>
  );
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
