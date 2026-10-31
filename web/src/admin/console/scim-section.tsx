// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";

import type { AdminSignInApi, SignInView } from "./api";

const DAY = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

/** The new token, shown once, with a way to copy it. */
function NewToken({
  token,
  onDone,
}: {
  readonly token: string;
  readonly onDone: () => void;
}) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  return (
    <div
      className="vl-admin-secret"
      role="region"
      aria-label={t("admin-scim-token-label")}
    >
      <p>
        <strong>{t("admin-scim-token-title")}</strong>{" "}
        {t("admin-scim-token-hint")}
      </p>
      <code className="vl-account-secret" data-testid="scim-token">
        {token}
      </code>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            navigator.clipboard.writeText(token).then(
              () => {
                setCopied(true);
              },
              () => {
                setCopied(false);
              },
            );
          }}
        >
          {copied ? t("tokens-copied") : t("admin-scim-copy")}
        </button>
        <button type="button" className="vl-filled-button" onClick={onDone}>
          {t("admin-scim-done")}
        </button>
      </div>
    </div>
  );
}

/** Admin → Sign-in → Provisioning: the SCIM URL and its token (D294). */
export function ScimSection({
  api,
  view,
  onChanged,
}: {
  readonly api: AdminSignInApi;
  readonly view: SignInView;
  readonly onChanged: (view: SignInView, notice: string) => void;
}) {
  const t = useT();
  const [token, setToken] = useState<string>();
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const scim = view.scim;
  const run = <T,>(work: Promise<T>, done: (value: T) => void) => {
    setBusy(true);
    setError(undefined);
    setConfirming(false);
    work
      .then(done, (error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };
  const rotate = () => {
    run(api.rotateScim(), (made) => {
      setToken(made.token);
      onChanged({ ...view, scim: made.scim }, t("admin-scim-made"));
    });
  };
  return (
    <section aria-labelledby="admin-sign-in-scim">
      <h2 id="admin-sign-in-scim">{t("admin-scim")}</h2>
      <p className="vl-muted">{t("admin-scim-intro")}</p>
      <p>
        {t("admin-scim-url")} <code>{scim.baseUrl}</code>
      </p>
      <p className="vl-muted">
        {scim.enabled &&
        scim.tokenCreatedAt !== null &&
        scim.tokenCreatedAt !== undefined
          ? t("admin-scim-on", {
              date: DAY.format(new Date(scim.tokenCreatedAt)),
            })
          : t("admin-scim-off")}
      </p>
      {token !== undefined && (
        <NewToken
          token={token}
          onDone={() => {
            setToken(undefined);
          }}
        />
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-admin-actions">
        {scim.enabled && !confirming && (
          <>
            <button
              type="button"
              className="vl-text-button"
              disabled={busy}
              onClick={() => {
                setConfirming(true);
              }}
            >
              {t("admin-scim-rotate")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              disabled={busy}
              onClick={() => {
                run(api.revokeScim(), (next) => {
                  setToken(undefined);
                  onChanged(next, t("admin-scim-turned-off"));
                });
              }}
            >
              {t("admin-scim-turn-off")}
            </button>
          </>
        )}
        {scim.enabled && confirming && (
          <>
            <span>{t("admin-scim-rotate-confirm")}</span>
            <button
              type="button"
              className="vl-filled-button"
              disabled={busy}
              onClick={rotate}
            >
              {t("admin-scim-rotate")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                setConfirming(false);
              }}
            >
              {t("factors-cancel")}
            </button>
          </>
        )}
        {!scim.enabled && (
          <button
            type="button"
            className="vl-filled-button"
            disabled={busy}
            onClick={rotate}
          >
            {t("admin-scim-turn-on")}
          </button>
        )}
      </div>
    </section>
  );
}
