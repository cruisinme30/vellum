// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../../i18n/i18n";

import type { UserView } from "./api";

/** Ending someone's sessions or second factors asks once more first. */
export function SignInHelp({
  user,
  busy,
  onSignInHelp,
}: {
  readonly user: UserView;
  readonly busy: boolean;
  readonly onSignInHelp: (help: "reset-factors" | "sign-out") => void;
}) {
  const t = useT();
  const [confirm, setConfirm] = useState<"reset-factors" | "sign-out">();
  if (confirm !== undefined) {
    const question =
      confirm === "sign-out"
        ? t("admin-users-sign-out-confirm", { name: user.name })
        : t("admin-users-reset-2fa-confirm", { name: user.name });
    return (
      <div className="vl-admin-row-actions" role="group" aria-label={question}>
        <span>{question}</span>
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy}
          onClick={() => {
            setConfirm(undefined);
            onSignInHelp(confirm);
          }}
        >
          {confirm === "sign-out"
            ? t("admin-users-sign-out")
            : t("admin-users-reset-2fa")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            setConfirm(undefined);
          }}
        >
          {t("admin-users-keep")}
        </button>
      </div>
    );
  }
  return (
    <div className="vl-admin-row-actions">
      <button
        type="button"
        className="vl-text-button"
        disabled={busy}
        aria-label={t("admin-users-sign-out-label", { name: user.name })}
        onClick={() => {
          setConfirm("sign-out");
        }}
      >
        {t("admin-users-sign-out")}
      </button>
      {user.secondFactor === true && (
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          aria-label={t("admin-users-reset-2fa-label", { name: user.name })}
          onClick={() => {
            setConfirm("reset-factors");
          }}
        >
          {t("admin-users-reset-2fa")}
        </button>
      )}
    </div>
  );
}
