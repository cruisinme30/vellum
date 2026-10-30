// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import "../../account/account.css";

import type { AdminTokensApi, AdminTokenView, TokenPolicy } from "./api";
import { AdminScreen } from "./context";

const DAY = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });

function when(value: string | null | undefined, fallback: string): string {
  return value === null || value === undefined ? fallback : DAY.format(new Date(value));
}

/** The cap on new tokens' expiry: a number of days, or none. */
function CapForm({
  api,
  policy,
  onSaved,
}: {
  readonly api: AdminTokensApi;
  readonly policy: TokenPolicy;
  readonly onSaved: (policy: TokenPolicy) => void;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [capped, setCapped] = useState(policy.maxDays !== null && policy.maxDays !== undefined);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const days = capped ? Number(data.get("days")) : undefined;
    setBusy(true);
    setError(undefined);
    api
      .setPolicy(days)
      .then(onSaved, (error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <form className="vl-admin-form" aria-labelledby={`${id}-title`} onSubmit={submit}>
      <h2 id={`${id}-title`}>{t("admin-tokens-cap-title")}</h2>
      <label className="vl-admin-toggle">
        <input
          type="checkbox"
          checked={capped}
          onChange={(event) => {
            setCapped(event.currentTarget.checked);
          }}
        />
        {t("admin-tokens-cap-on")}
      </label>
      <label>
        {t("admin-tokens-cap-days")}
        <input
          name="days"
          type="number"
          min={1}
          max={3650}
          required={capped}
          disabled={!capped}
          defaultValue={policy.maxDays ?? 90}
        />
        <small className="vl-muted">{t("admin-tokens-cap-hint")}</small>
      </label>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {busy ? t("admin-saving") : t("admin-save")}
        </button>
      </div>
    </form>
  );
}

function Row({
  token,
  busy,
  onRevoke,
}: {
  readonly token: AdminTokenView;
  readonly busy: boolean;
  readonly onRevoke: () => void;
}) {
  const t = useT();
  const [confirm, setConfirm] = useState(false);
  const owner = token.owner?.name ?? t("admin-tokens-no-owner");
  return (
    <tr>
      <th scope="row">{token.token.name}</th>
      <td>
        {owner}
        {token.owner !== null && token.owner !== undefined && (
          <div className="vl-muted">{token.owner.email}</div>
        )}
      </td>
      <td>
        {token.token.scopes.map((scope) => (
          <code key={scope} className="vl-account-chip">
            {scope}
          </code>
        ))}
      </td>
      <td>{when(token.token.lastUsedAt, t("tokens-never-used"))}</td>
      <td>{when(token.token.expiresAt, t("tokens-no-expiry"))}</td>
      <td>
        {confirm ? (
          <div className="vl-admin-row-actions">
            <span>{t("admin-tokens-revoke-confirm", { name: token.token.name, owner })}</span>
            <button
              type="button"
              className="vl-filled-button"
              disabled={busy}
              onClick={() => {
                setConfirm(false);
                onRevoke();
              }}
            >
              {t("tokens-revoke")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                setConfirm(false);
              }}
            >
              {t("tokens-keep")}
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            aria-label={t("admin-tokens-revoke-label", { name: token.token.name, owner })}
            onClick={() => {
              setConfirm(true);
            }}
          >
            {t("tokens-revoke")}
          </button>
        )}
      </td>
    </tr>
  );
}

function Tokens({ api }: { readonly api: AdminTokensApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState<string>();
  const [notice, setNotice] = useState<{ ok: boolean; message: string }>();
  const loaded = useLoaded(
    async () => {
      const [tokens, policy] = await Promise.all([api.list(), api.policy()]);
      return { tokens, policy };
    },
    [api, reload],
    failureText,
  );
  const revoke = (token: AdminTokenView) => {
    setBusy(token.token.id);
    setNotice(undefined);
    api
      .revoke(token.token.id)
      .then(
        () => {
          setNotice({ ok: true, message: t("tokens-revoked", { name: token.token.name }) });
          setReload((n) => n + 1);
        },
        (error: unknown) => {
          setNotice({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(undefined);
      });
  };
  return (
    <>
      <div role="status">{notice?.ok === true && <p>{notice.message}</p>}</div>
      {notice?.ok === false && (
        <p className="vl-field-error" role="alert">
          {notice.message}
        </p>
      )}
      {loaded.state === "loading" && <p className="vl-muted">{t("admin-loading")}</p>}
      {loaded.state === "failed" && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
        </p>
      )}
      {loaded.state === "ready" && (
        <>
          {loaded.value.tokens.length === 0 ? (
            <p className="vl-muted">{t("admin-tokens-empty")}</p>
          ) : (
            <div className="vl-admin-scroll">
              <table className="vl-admin-table">
                <caption>{t("tokens-caption", { count: loaded.value.tokens.length })}</caption>
                <thead>
                  <tr>
                    <th scope="col">{t("tokens-name")}</th>
                    <th scope="col">{t("admin-tokens-owner")}</th>
                    <th scope="col">{t("tokens-scopes")}</th>
                    <th scope="col">{t("tokens-last-used")}</th>
                    <th scope="col">{t("tokens-expires")}</th>
                    <th scope="col">{t("tokens-actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {loaded.value.tokens.map((token) => (
                    <Row
                      key={token.token.id}
                      token={token}
                      busy={busy === token.token.id}
                      onRevoke={() => {
                        revoke(token);
                      }}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <CapForm
            api={api}
            policy={loaded.value.policy}
            onSaved={(policy) => {
              setNotice({
                ok: true,
                message:
                  policy.maxDays === null || policy.maxDays === undefined
                    ? t("admin-tokens-cap-off-saved")
                    : t("admin-tokens-cap-saved", { days: policy.maxDays }),
              });
            }}
          />
        </>
      )}
    </>
  );
}

/** Admin → API tokens: everyone's tokens, revoking any, and the expiry cap. */
export function AdminTokensPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-tokens")} intro={t("admin-tokens-intro")}>
      {(api) =>
        api.tokens === undefined ? <p>{t("admin-needs-server")}</p> : <Tokens api={api.tokens} />
      }
    </AdminScreen>
  );
}
