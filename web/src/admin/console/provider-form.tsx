// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";

import {
  formText,
  type AdminSignInApi,
  type ProviderInput,
  type ProviderView,
} from "./api";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;
type Kind = "oidc" | "saml";

function domainList(text: string): string[] {
  return text
    .split(/[\s,]+/u)
    .map((d) => d.trim())
    .filter((d) => d.length > 0);
}

/**
 * What the form holds. A blank client secret or certificate keeps the
 * stored one when changing a provider.
 */
function providerInput(
  data: FormData,
  kind: Kind,
  adding: boolean,
): ProviderInput {
  const common = {
    kind,
    name: formText(data, "name"),
    allowedDomains: domainList(formText(data, "domains")),
    autoCreate: data.get("autoCreate") === "on",
    enabled: data.get("enabled") === "on",
  };
  if (kind === "saml") {
    return {
      ...common,
      entityId: formText(data, "entityId"),
      ssoUrl: formText(data, "ssoUrl"),
      certificate: formText(data, "certificate"),
      metadataUrl: formText(data, "metadataUrl"),
    };
  }
  const secret = formText(data, "secret");
  return {
    ...common,
    issuer: formText(data, "issuer"),
    clientId: formText(data, "clientId"),
    ...(secret.length > 0 || adding ? { clientSecret: secret } : {}),
  };
}

/** A labelled text field. */
function Field({
  id,
  name,
  label,
  ...rest
}: {
  readonly id: string;
  readonly name: string;
  readonly label: string;
} & Omit<ComponentProps<"input">, "id" | "name">) {
  return (
    <>
      <label htmlFor={id}>{label}</label>
      <input id={id} name={name} {...rest} />
    </>
  );
}

function OidcFields({
  field,
  editing,
}: {
  readonly field: (name: string) => string;
  readonly editing?: ProviderView | undefined;
}) {
  const t = useT();
  return (
    <>
      <Field
        id={field("issuer")}
        name="issuer"
        label={t("admin-sso-issuer")}
        type="url"
        required
        defaultValue={editing?.issuer ?? ""}
        placeholder="https://northstar.okta.com"
      />
      <Field
        id={field("clientId")}
        name="clientId"
        label={t("admin-sso-client-id")}
        required
        defaultValue={editing?.clientId ?? ""}
      />
      <Field
        id={field("secret")}
        name="secret"
        label={t("admin-sso-client-secret")}
        type="password"
        autoComplete="off"
        placeholder={
          editing?.hasClientSecret === true
            ? t("admin-sso-secret-kept")
            : undefined
        }
      />
    </>
  );
}

function samlStart(editing: ProviderView | undefined) {
  return {
    metadataUrl: editing?.metadataUrl ?? "",
    entityId: editing?.entityId ?? "",
    ssoUrl: editing?.ssoUrl ?? "",
    fingerprints: editing?.certificateFingerprints ?? [],
  };
}

function ReadNotice({
  ok,
  text,
}: {
  readonly ok: boolean;
  readonly text: string;
}) {
  return ok ? (
    <p className="vl-muted" role="status">
      {text}
    </p>
  ) : (
    <p className="vl-field-error" role="alert">
      {text}
    </p>
  );
}

/** The fields every provider has, as the form starts. */
function commonStart(editing: ProviderView | undefined) {
  const kind: Kind = editing?.kind === "saml" ? "saml" : "oidc";
  return {
    kind,
    name: editing?.name ?? "",
    domains: editing?.allowedDomains.join(", ") ?? "",
    autoCreate: editing?.autoCreate ?? false,
    enabled: editing?.enabled ?? true,
  };
}

/** SAML settings, typed in or read from the provider's metadata URL. */
function SamlFields({
  api,
  field,
  editing,
}: {
  readonly api: AdminSignInApi;
  readonly field: (name: string) => string;
  readonly editing?: ProviderView | undefined;
}) {
  const t = useT();
  const start = samlStart(editing);
  const [metadataUrl, setMetadataUrl] = useState(start.metadataUrl);
  const [entityId, setEntityId] = useState(start.entityId);
  const [ssoUrl, setSsoUrl] = useState(start.ssoUrl);
  const [certificate, setCertificate] = useState("");
  const [reading, setReading] = useState(false);
  const [notice, setNotice] = useState<{ ok: boolean; text: string }>();
  const fingerprints = start.fingerprints;
  const read = () => {
    setReading(true);
    setNotice(undefined);
    api
      .samlMetadata(metadataUrl)
      .then(
        (found) => {
          setEntityId(found.entityId);
          setSsoUrl(found.ssoUrl);
          setCertificate(found.certificate);
          setNotice({
            ok: true,
            text: t("admin-saml-read", {
              count: found.certificateFingerprints.length,
            }),
          });
        },
        (error: unknown) => {
          setNotice({ ok: false, text: failureText(error) });
        },
      )
      .finally(() => {
        setReading(false);
      });
  };
  return (
    <>
      <Field
        id={field("metadataUrl")}
        name="metadataUrl"
        label={t("admin-saml-metadata-url")}
        type="url"
        value={metadataUrl}
        placeholder="https://northstar.okta.com/app/…/sso/saml/metadata"
        onChange={(event) => {
          setMetadataUrl(event.target.value);
        }}
      />
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          disabled={reading || metadataUrl.trim().length === 0}
          onClick={read}
        >
          {t("admin-saml-read-button")}
        </button>
      </div>
      {notice !== undefined && <ReadNotice ok={notice.ok} text={notice.text} />}
      <Field
        id={field("entityId")}
        name="entityId"
        label={t("admin-saml-entity-id")}
        required
        value={entityId}
        onChange={(event) => {
          setEntityId(event.target.value);
        }}
      />
      <Field
        id={field("ssoUrl")}
        name="ssoUrl"
        label={t("admin-saml-sso-url")}
        type="url"
        required
        value={ssoUrl}
        onChange={(event) => {
          setSsoUrl(event.target.value);
        }}
      />
      <label htmlFor={field("certificate")}>
        {t("admin-saml-certificate")}
      </label>
      <textarea
        id={field("certificate")}
        name="certificate"
        rows={4}
        spellCheck={false}
        required={fingerprints.length === 0}
        value={certificate}
        placeholder={
          fingerprints.length > 0
            ? t("admin-saml-certificate-kept")
            : "-----BEGIN CERTIFICATE-----"
        }
        onChange={(event) => {
          setCertificate(event.target.value);
        }}
      />
      {fingerprints.map((fingerprint) => (
        <p key={fingerprint} className="vl-muted">
          {t("admin-saml-fingerprint")} <code>{fingerprint}</code>
        </p>
      ))}
    </>
  );
}

function KindChoice({
  kind,
  fixed,
  onChange,
}: {
  readonly kind: Kind;
  readonly fixed: boolean;
  readonly onChange: (kind: Kind) => void;
}) {
  const t = useT();
  const options: readonly (readonly [Kind, string])[] = [
    ["oidc", t("admin-sso-kind-oidc")],
    ["saml", t("admin-sso-kind-saml")],
  ];
  return (
    <fieldset className="vl-admin-fieldset">
      <legend>{t("admin-sso-kind")}</legend>
      {options.map(([value, label]) => (
        <label key={value} className="vl-account-scope">
          <input
            type="radio"
            name="kind"
            value={value}
            checked={kind === value}
            disabled={fixed}
            onChange={() => {
              onChange(value);
            }}
          />
          <span>{label}</span>
        </label>
      ))}
    </fieldset>
  );
}

/** The fields of one provider, for adding or changing it. */
export function ProviderForm({
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
  const start = commonStart(editing);
  const [kind, setKind] = useState<Kind>(start.kind);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const input = providerInput(
      new FormData(form),
      kind,
      editing === undefined,
    );
    setBusy(true);
    setError(undefined);
    const saving =
      editing === undefined
        ? api.addProvider(input)
        : api.updateProvider(editing.id, input);
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
  return (
    <form
      className="vl-admin-form"
      aria-labelledby={field("title")}
      onSubmit={submit}
      aria-busy={busy}
    >
      <h3 id={field("title")}>
        {editing === undefined
          ? t("admin-sso-add")
          : t("admin-sso-edit", { name: editing.name })}
      </h3>
      <KindChoice
        kind={kind}
        fixed={editing !== undefined}
        onChange={setKind}
      />
      <Field
        id={field("name")}
        name="name"
        label={t("admin-sso-name")}
        required
        maxLength={60}
        defaultValue={start.name}
        placeholder="Okta"
      />
      {kind === "saml" ? (
        <SamlFields api={api} field={field} editing={editing} />
      ) : (
        <OidcFields field={field} editing={editing} />
      )}
      <Field
        id={field("domains")}
        name="domains"
        label={t("admin-sso-domains")}
        defaultValue={start.domains}
        placeholder="northstar.dev"
      />
      <label className="vl-account-scope">
        <input
          type="checkbox"
          name="autoCreate"
          defaultChecked={start.autoCreate}
        />
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
