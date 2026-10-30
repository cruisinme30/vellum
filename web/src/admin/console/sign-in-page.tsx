// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";
import { Link } from "react-router";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import type { AdminSignInApi, SignInView } from "./api";
import { AdminScreen } from "./context";
import { ProvidersSection } from "./providers-section";
import { ScimSection } from "./scim-section";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;
type Policy = SignInView["secondFactor"];

const POLICIES: readonly (readonly [Policy, string, string])[] = [
  ["optional", "admin-sign-in-2fa-optional", "admin-sign-in-2fa-optional-hint"],
  ["admins", "admin-sign-in-2fa-admins", "admin-sign-in-2fa-admins-hint"],
  ["everyone", "admin-sign-in-2fa-everyone", "admin-sign-in-2fa-everyone-hint"],
];

/** Who must use an authenticator app or passkey with their password. */
function SecondFactorForm({
  api,
  view,
  onSaved,
}: {
  readonly api: AdminSignInApi;
  readonly view: SignInView;
  readonly onSaved: (view: SignInView) => void;
}) {
  const t = useT();
  const id = useId();
  const [policy, setPolicy] = useState<Policy>(view.secondFactor);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: Submit = (event) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    api
      .update({ secondFactor: policy })
      .then(onSaved, (error_: unknown) => {
        setError(failureText(error_));
      })
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <form className="vl-admin-form" aria-labelledby={`${id}-title`} onSubmit={submit}>
      <h2 id={`${id}-title`}>{t("admin-sign-in-2fa")}</h2>
      <fieldset className="vl-admin-fieldset">
        <legend>{t("admin-sign-in-2fa-legend")}</legend>
        {POLICIES.map(([value, label, hint]) => (
          <label key={value} className="vl-account-scope">
            <input
              type="radio"
              name="second-factor"
              value={value}
              checked={policy === value}
              onChange={() => {
                setPolicy(value);
              }}
            />
            <span>
              {t(label)}
              <small className="vl-muted">{t(hint)}</small>
            </span>
          </label>
        ))}
      </fieldset>
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

function SignIn({ api }: { readonly api: AdminSignInApi }) {
  const t = useT();
  const [saved, setSaved] = useState<SignInView>();
  const [notice, setNotice] = useState<string>();
  const loaded = useLoaded(() => api.settings(), [api], failureText);
  if (loaded.state === "loading") return <p className="vl-muted">{t("admin-loading")}</p>;
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
      </p>
    );
  }
  const view = saved ?? loaded.value;
  return (
    <>
      <div role="status">{notice !== undefined && <p>{notice}</p>}</div>
      <SecondFactorForm
        api={api}
        view={view}
        onSaved={(next) => {
          setSaved(next);
          setNotice(t("admin-sign-in-saved"));
        }}
      />
      <ProvidersSection
        api={api}
        view={view}
        onChanged={(next, message) => {
          setSaved(next);
          setNotice(message);
        }}
      />
      <ScimSection
        api={api}
        view={view}
        onChanged={(next, message) => {
          setSaved(next);
          setNotice(message);
        }}
      />
      <section aria-labelledby="admin-sign-in-tokens">
        <h2 id="admin-sign-in-tokens">{t("admin-tokens")}</h2>
        <p className="vl-muted">
          {view.maxTokenDays === null || view.maxTokenDays === undefined
            ? t("admin-sign-in-tokens-uncapped")
            : t("admin-sign-in-tokens-capped", { days: view.maxTokenDays })}{" "}
          <Link to="/admin/tokens">{t("admin-sign-in-tokens-link")}</Link>
        </p>
      </section>
    </>
  );
}

/** Admin → Sign-in: how people sign in to the workspace. */
export function SignInPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-sign-in")} intro={t("admin-sign-in-intro")}>
      {(api) =>
        api.signIn === undefined ? <p>{t("admin-needs-server")}</p> : <SignIn api={api.signIn} />
      }
    </AdminScreen>
  );
}
