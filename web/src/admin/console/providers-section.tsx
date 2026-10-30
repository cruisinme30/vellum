// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";
import { Link } from "react-router";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";

import {
  formText,
  type AdminSignInApi,
  type ProviderInput,
  type ProviderView,
  type SignInView,
} from "./api";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;
type Changed = (view: SignInView, notice: string) => void;

function domainList(text: string): string[] {
  return text
    .split(/[\s,]+/u)
    .map((d) => d.trim())
    .filter((d) => d.length > 0);
}

/** What the form holds; a blank secret keeps the stored one when changing. */
function providerInput(data: FormData, adding: boolean): ProviderInput {
  const secret = formText(data, "secret");
  return {
    name: formText(data, "name"),
    issuer: formText(data, "issuer"),
    clientId: formText(data, "clientId"),
    allowedDomains: domainList(formText(data, "domains")),
    autoCreate: data.get("autoCreate") === "on",
    enabled: data.get("enabled") === "on",
    ...(secret.length > 0 || adding ? { clientSecret: secret } : {}),
  };
}

const NEW_PROVIDER = {
  name: "",
  issuer: "",
  clientId: "",
  domains: "",
  autoCreate: false,
  enabled: true,
  hasSecret: false,
};

/** The form's starting values. */
function defaults(editing: ProviderView | undefined): typeof NEW_PROVIDER {
  if (editing === undefined) return NEW_PROVIDER;
  return {
    name: editing.name,
    issuer: editing.issuer ?? "",
    clientId: editing.clientId ?? "",
    domains: editing.allowedDomains.join(", "),
    autoCreate: editing.autoCreate,
    enabled: editing.enabled,
    hasSecret: editing.hasClientSecret,
  };
}

/** The fields of one provider, for adding or changing it. */
function ProviderForm({
  api,
  editing,
  onSaved,
  onCancel,
}: {
  readonly api: AdminSignInApi;
  readonly editing?: ProviderView | undefined;
  readonly onSaved: (provider: ProviderView) => void;
  readonly onCancel?: (() => void) | undefined;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const input = providerInput(new FormData(form), editing === undefined);
    setBusy(true);
    setError(undefined);
    const saving =
      editing === undefined ? api.addProvider(input) : api.updateProvider(editing.id, input);
    saving
      .then(
        (saved) => {
          if (editing === undefined) form.reset();
          onSaved(saved);
        },
        (error_: unknown) => {
          setError(failureText(error_));
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  const field = (name: string) => `${id}-${name}`;
  const start = defaults(editing);
  return (
    <form
      className="vl-admin-form"
      aria-labelledby={field("title")}
      onSubmit={submit}
      aria-busy={busy}
    >
      <h3 id={field("title")}>
        {editing === undefined ? t("admin-sso-add") : t("admin-sso-edit", { name: editing.name })}
      </h3>
      <label htmlFor={field("name")}>{t("admin-sso-name")}</label>
      <input
        id={field("name")}
        name="name"
        required
        maxLength={60}
        defaultValue={start.name}
        placeholder="Okta"
      />
      <label htmlFor={field("issuer")}>{t("admin-sso-issuer")}</label>
      <input
        id={field("issuer")}
        name="issuer"
        type="url"
        required
        defaultValue={start.issuer}
        placeholder="https://northstar.okta.com"
      />
      <label htmlFor={field("clientId")}>{t("admin-sso-client-id")}</label>
      <input id={field("clientId")} name="clientId" required defaultValue={start.clientId} />
      <label htmlFor={field("secret")}>{t("admin-sso-client-secret")}</label>
      <input
        id={field("secret")}
        name="secret"
        type="password"
        autoComplete="off"
        placeholder={start.hasSecret ? t("admin-sso-secret-kept") : undefined}
      />
      <label htmlFor={field("domains")}>{t("admin-sso-domains")}</label>
      <input
        id={field("domains")}
        name="domains"
        defaultValue={start.domains}
        placeholder="northstar.dev"
      />
      <label className="vl-account-scope">
        <input type="checkbox" name="autoCreate" defaultChecked={start.autoCreate} />
        <span>{t("admin-sso-auto-create")}</span>
      </label>
      <label className="vl-account-scope">
        <input type="checkbox" name="enabled" defaultChecked={start.enabled} />
        <span>{t("admin-sso-enabled")}</span>
      </label>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-admin-actions">
        {onCancel !== undefined && (
          <button type="button" className="vl-text-button" onClick={onCancel}>
            {t("factors-cancel")}
          </button>
        )}
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {busy ? t("admin-saving") : t("admin-save")}
        </button>
      </div>
    </form>
  );
}

/** Refuse passwords, except for break-glass admins. */
function RequireSsoForm({
  api,
  view,
  onChanged,
}: {
  readonly api: AdminSignInApi;
  readonly view: SignInView;
  readonly onChanged: Changed;
}) {
  const t = useT();
  const id = useId();
  const [on, setOn] = useState(view.requireSso);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: Submit = (event) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    api
      .update({ requireSso: on })
      .then(
        (next) => {
          onChanged(next, t("admin-sign-in-saved"));
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
    <form className="vl-admin-form" onSubmit={submit} aria-labelledby={`${id}-title`}>
      <h3 id={`${id}-title`}>{t("admin-sso-require-title")}</h3>
      <label className="vl-account-scope">
        <input
          type="checkbox"
          checked={on}
          onChange={(event) => {
            setOn(event.target.checked);
          }}
        />
        <span>
          {t("admin-sso-require")}
          <small className="vl-muted">
            {t("admin-sso-require-hint", { count: view.breakGlassAdmins })}{" "}
            <Link to="/admin/users">{t("admin-sso-require-users")}</Link>
          </small>
        </span>
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

function ProviderRow({
  provider,
  busy,
  onEdit,
  onRemove,
}: {
  readonly provider: ProviderView;
  readonly busy: boolean;
  readonly onEdit: () => void;
  readonly onRemove: () => void;
}) {
  const t = useT();
  return (
    <tr>
      <th scope="row">{provider.name}</th>
      <td>
        <code>{provider.issuer}</code>
      </td>
      <td>
        {provider.enabled ? t("factors-on") : t("admin-sso-off")}
        {provider.readOnly && <div className="vl-muted">{t("admin-sso-from-env")}</div>}
      </td>
      <td>
        {!provider.readOnly && (
          <div className="vl-admin-row-actions">
            <button
              type="button"
              className="vl-text-button"
              disabled={busy}
              aria-label={t("admin-sso-edit", { name: provider.name })}
              onClick={onEdit}
            >
              {t("admin-sso-edit-short")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              disabled={busy}
              aria-label={t("admin-sso-remove-label", { name: provider.name })}
              onClick={onRemove}
            >
              {t("factors-remove")}
            </button>
          </div>
        )}
      </td>
    </tr>
  );
}

/** Admin → Sign-in → Single sign-on: providers and "require SSO". */
export function ProvidersSection({
  api,
  view,
  onChanged,
}: {
  readonly api: AdminSignInApi;
  readonly view: SignInView;
  readonly onChanged: Changed;
}) {
  const t = useT();
  const [editing, setEditing] = useState<ProviderView>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const refresh = (notice: string) => {
    api.settings().then(
      (next) => {
        onChanged(next, notice);
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  const remove = (provider: ProviderView) => {
    setBusy(true);
    setError(undefined);
    api
      .removeProvider(provider.id)
      .then(
        (next) => {
          onChanged(next, t("admin-sso-removed", { name: provider.name }));
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
    <section aria-labelledby="admin-sign-in-sso">
      <h2 id="admin-sign-in-sso">{t("admin-sso")}</h2>
      <p className="vl-muted">
        {t("admin-sso-callback")} <code>{view.oidcCallbackUrl}</code>
      </p>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      {view.providers.length === 0 ? (
        <p className="vl-muted">{t("admin-sso-none")}</p>
      ) : (
        <div className="vl-admin-scroll">
          <table className="vl-admin-table">
            <caption>{t("admin-sso-caption", { count: view.providers.length })}</caption>
            <thead>
              <tr>
                <th scope="col">{t("admin-sso-name")}</th>
                <th scope="col">{t("admin-sso-issuer")}</th>
                <th scope="col">{t("admin-sso-status")}</th>
                <th scope="col">{t("tokens-actions")}</th>
              </tr>
            </thead>
            <tbody>
              {view.providers.map((provider) => (
                <ProviderRow
                  key={provider.id}
                  provider={provider}
                  busy={busy}
                  onEdit={() => {
                    setEditing(provider);
                  }}
                  onRemove={() => {
                    remove(provider);
                  }}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
      <ProviderForm
        key={editing?.id ?? "new"}
        api={api}
        editing={editing}
        onCancel={
          editing === undefined
            ? undefined
            : () => {
                setEditing(undefined);
              }
        }
        onSaved={(saved) => {
          setEditing(undefined);
          refresh(t("admin-sso-saved", { name: saved.name }));
        }}
      />
      <RequireSsoForm key={String(view.requireSso)} api={api} view={view} onChanged={onChanged} />
    </section>
  );
}
