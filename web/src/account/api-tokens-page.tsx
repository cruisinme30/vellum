// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import { formText } from "../admin/console/api";
import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import "../admin/console/console.css";
import "./account.css";

import type {
  AccountApi,
  CreatedToken,
  Scope,
  TokenPolicy,
  TokenView,
} from "./api";

const DAY = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

/** Each scope's label and what it lets a token do. */
const SCOPE_TEXT: Readonly<
  Record<Scope, readonly [label: string, hint: string]>
> = {
  read: ["tokens-scope-read", "tokens-scope-read-hint"],
  "write:draft": ["tokens-scope-write-draft", "tokens-scope-write-draft-hint"],
  "write:direct": [
    "tokens-scope-write-direct",
    "tokens-scope-write-direct-hint",
  ],
  work: ["tokens-scope-work", "tokens-scope-work-hint"],
  admin: ["tokens-scope-admin", "tokens-scope-admin-hint"],
};

const PRESETS = [7, 30, 90, 365] as const;

/**
 * The expiry choices: the presets within the workspace's cap, the cap
 * itself, and "never" only when there is no cap.
 */
export function expiryChoices(maxDays?: number | null): (number | "never")[] {
  if (maxDays === null || maxDays === undefined) return [...PRESETS, "never"];
  const within: number[] = PRESETS.filter((days) => days <= maxDays);
  return within.includes(maxDays) ? within : [...within, maxDays];
}

/** The default expiry: 30 days, or the cap when it is shorter. */
export function defaultExpiry(maxDays?: number | null): number {
  return maxDays === null || maxDays === undefined ? 30 : Math.min(30, maxDays);
}

/** A token's secret, shown once right after it is created. */
function NewSecret({
  created,
  onDone,
}: {
  readonly created: CreatedToken;
  readonly onDone: () => void;
}) {
  const t = useT();
  const id = useId();
  const [copied, setCopied] = useState(false);
  return (
    <div className="vl-admin-secret" role="region" aria-labelledby={id}>
      <p id={id}>
        <strong>
          {t("tokens-secret-title", { name: created.details.name })}
        </strong>{" "}
        {t("tokens-secret-hint")}
      </p>
      <code className="vl-admin-secret-value" data-testid="new-token">
        {created.token}
      </code>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            navigator.clipboard.writeText(created.token).then(
              () => {
                setCopied(true);
              },
              () => {
                setCopied(false);
              },
            );
          }}
        >
          {copied ? t("tokens-copied") : t("tokens-copy")}
        </button>
        <button type="button" className="vl-filled-button" onClick={onDone}>
          {t("tokens-secret-done")}
        </button>
      </div>
    </div>
  );
}

function CreateTokenForm({
  api,
  policy,
  onCreated,
}: {
  readonly api: AccountApi;
  readonly policy: TokenPolicy;
  readonly onCreated: (created: CreatedToken) => void;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const choices = expiryChoices(policy.maxDays);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const name = formText(data, "name");
    const scopes = policy.scopes.filter(
      (scope) => data.get(`scope-${scope}`) === "on",
    );
    if (scopes.length === 0) {
      setError(t("tokens-error-scopes"));
      return;
    }
    const expiry = formText(data, "expiry");
    setBusy(true);
    setError(undefined);
    api
      .createToken({
        name,
        scopes,
        ...(expiry === "never" ? {} : { expiresInDays: Number(expiry) }),
      })
      .then(
        (created) => {
          form.reset();
          onCreated(created);
        },
        (error_: unknown) => {
          setError(failureText(error_));
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <form
      className="vl-admin-form"
      aria-labelledby={`${id}-title`}
      onSubmit={submit}
    >
      <h2 id={`${id}-title`}>{t("tokens-create-title")}</h2>
      <label>
        {t("tokens-name")}
        <input
          name="name"
          required
          maxLength={120}
          autoComplete="off"
          placeholder={t("tokens-name-placeholder")}
        />
      </label>
      <fieldset className="vl-admin-fieldset">
        <legend>{t("tokens-scopes")}</legend>
        {policy.scopes.map((scope) => {
          const [label, hint] = SCOPE_TEXT[scope];
          return (
            <label key={scope} className="vl-account-scope">
              <input
                type="checkbox"
                name={`scope-${scope}`}
                defaultChecked={scope === "read"}
              />
              <span>
                <code>{scope}</code> {t(label)}
                <small className="vl-muted">{t(hint)}</small>
              </span>
            </label>
          );
        })}
      </fieldset>
      <label>
        {t("tokens-expiry")}
        <select
          name="expiry"
          defaultValue={String(defaultExpiry(policy.maxDays))}
        >
          {choices.map((choice) => (
            <option key={choice} value={String(choice)}>
              {choice === "never"
                ? t("tokens-expiry-never")
                : t("tokens-expiry-days", { days: choice })}
            </option>
          ))}
        </select>
        {policy.maxDays !== null && policy.maxDays !== undefined && (
          <small className="vl-muted">
            {t("tokens-expiry-cap", { days: policy.maxDays })}
          </small>
        )}
      </label>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {busy ? t("tokens-creating") : t("tokens-create")}
        </button>
      </div>
    </form>
  );
}

function when(value: string | null | undefined, fallback: string): string {
  return value === null || value === undefined
    ? fallback
    : DAY.format(new Date(value));
}

function TokenRow({
  token,
  now,
  busy,
  onRevoke,
}: {
  readonly token: TokenView;
  /** When the list was loaded, in milliseconds. */
  readonly now: number;
  readonly busy: boolean;
  readonly onRevoke: () => void;
}) {
  const t = useT();
  const [confirm, setConfirm] = useState(false);
  const expired =
    token.expiresAt !== null &&
    token.expiresAt !== undefined &&
    Date.parse(token.expiresAt) <= now;
  return (
    <tr>
      <th scope="row">{token.name}</th>
      <td>
        {token.scopes.map((scope) => (
          <code key={scope} className="vl-account-chip">
            {scope}
          </code>
        ))}
      </td>
      <td>{when(token.createdAt, "")}</td>
      <td>{when(token.lastUsedAt, t("tokens-never-used"))}</td>
      <td>
        {expired ? (
          <span className="vl-admin-badge vl-admin-badge-off">
            {t("tokens-expired")}
          </span>
        ) : (
          when(token.expiresAt, t("tokens-no-expiry"))
        )}
      </td>
      <td>
        {confirm ? (
          <div
            className="vl-admin-row-actions"
            role="group"
            aria-label={t("tokens-revoke-confirm", { name: token.name })}
          >
            <span>{t("tokens-revoke-confirm", { name: token.name })}</span>
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
            aria-label={t("tokens-revoke-label", { name: token.name })}
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

function Tokens({ api }: { readonly api: AccountApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [created, setCreated] = useState<CreatedToken>();
  const [busy, setBusy] = useState<string>();
  const [notice, setNotice] = useState<{ ok: boolean; message: string }>();
  const loaded = useLoaded(
    async () => {
      const [tokens, policy] = await Promise.all([
        api.tokens(),
        api.tokenPolicy(),
      ]);
      return { tokens, policy, now: Date.now() };
    },
    [api, reload],
    failureText,
  );
  const revoke = (token: TokenView) => {
    setBusy(token.id);
    setNotice(undefined);
    api
      .revokeToken(token.id)
      .then(
        () => {
          setNotice({
            ok: true,
            message: t("tokens-revoked", { name: token.name }),
          });
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
    <Ready loaded={loaded}>
      {({ tokens, policy, now }) => (
        <>
          <div role="status">
            {notice?.ok === true && <p>{notice.message}</p>}
          </div>
          {notice?.ok === false && (
            <p className="vl-field-error" role="alert">
              {notice.message}
            </p>
          )}
          {created !== undefined && (
            <NewSecret
              created={created}
              onDone={() => {
                setCreated(undefined);
              }}
            />
          )}
          {tokens.length === 0 ? (
            <p className="vl-muted">{t("tokens-empty")}</p>
          ) : (
            <div className="vl-admin-scroll">
              <table className="vl-admin-table">
                <caption>
                  {t("tokens-caption", { count: tokens.length })}
                </caption>
                <thead>
                  <tr>
                    <th scope="col">{t("tokens-name")}</th>
                    <th scope="col">{t("tokens-scopes")}</th>
                    <th scope="col">{t("tokens-created")}</th>
                    <th scope="col">{t("tokens-last-used")}</th>
                    <th scope="col">{t("tokens-expires")}</th>
                    <th scope="col">{t("tokens-actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {tokens.map((token) => (
                    <TokenRow
                      key={token.id}
                      token={token}
                      now={now}
                      busy={busy === token.id}
                      onRevoke={() => {
                        revoke(token);
                      }}
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <CreateTokenForm
            api={api}
            policy={policy}
            onCreated={(next) => {
              setCreated(next);
              setNotice(undefined);
              setReload((n) => n + 1);
            }}
          />
        </>
      )}
    </Ready>
  );
}

/** `/settings/api-tokens`: personal API tokens for MCP clients and scripts. */
export function ApiTokensPage() {
  const t = useT();
  return (
    <ServerScreen api="account" title={t("tokens-title")} className="vl-admin">
      {(api) => (
        <>
          <p className="vl-muted">{t("tokens-intro")}</p>
          <Tokens api={api} />
        </>
      )}
    </ServerScreen>
  );
}
