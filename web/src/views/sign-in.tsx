// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ReactNode } from "react";
import { Navigate, useNavigate, useSearchParams } from "react-router";

import { SecondStepRequired } from "../api/auth";
import { ApiError } from "../api/http";
import { safeNext, useSession } from "../api/session";
import { useT } from "../i18n/i18n";

import { SecondStep } from "./second-step";

/** What a failed request says, with the request ID to quote when reporting it. */
export function describeError(error: unknown, fallback: string): string {
  if (error instanceof ApiError) {
    return error.requestId === undefined
      ? error.message
      : `${error.message} (request ${error.requestId})`;
  }
  return fallback;
}

function AuthCard({
  title,
  intro,
  children,
}: {
  readonly title: string;
  readonly intro: string;
  readonly children: ReactNode;
}) {
  return (
    <main id="vl-main" className="vl-auth">
      <div className="vl-auth-card">
        <p className="vl-auth-brand">Vellum</p>
        <h1>{title}</h1>
        <p className="vl-muted">{intro}</p>
        {children}
      </div>
    </main>
  );
}

function Field({
  label,
  type,
  autoComplete,
  value,
  onChange,
  hint,
  minLength,
}: {
  readonly label: string;
  readonly type: "email" | "password" | "text";
  readonly autoComplete: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly hint?: string;
  readonly minLength?: number;
}) {
  const id = useId();
  return (
    <div className="vl-auth-field">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        type={type}
        autoComplete={autoComplete}
        required
        minLength={minLength}
        value={value}
        aria-describedby={hint === undefined ? undefined : `${id}-hint`}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      />
      {hint === undefined ? undefined : (
        <small id={`${id}-hint`} className="vl-muted">
          {hint}
        </small>
      )}
    </div>
  );
}

/** Runs a form submission, keeping its busy and error state. */
function useSubmit(fallback: string) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const submit = (run: () => Promise<void>) => (event: { preventDefault(): void }) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    run().then(
      () => {
        setBusy(false);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(describeError(error_, fallback));
      },
    );
  };
  return { busy, error, submit };
}

/** `/sign-in?next=…`: the server sends people here when they need to sign in. */
export function SignInView() {
  const t = useT();
  const session = useSession();
  const navigate = useNavigate();
  const [parameters] = useSearchParams();
  const next = safeNext(parameters.get("next"));
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [step, setStep] = useState<SecondStepRequired>();
  const { busy, error, submit } = useSubmit(t("auth-error"));

  if (session === undefined) return <Navigate to="/" replace />;
  if (session.status.needsSetup) return <Navigate to="/setup" replace />;

  const signedIn = async () => {
    await session.refresh();
    // A server-side page (an app asking for consent) needs a full load.
    if (next.startsWith("/oauth/")) globalThis.location.assign(next);
    else void navigate(next, { replace: true });
  };
  const onSubmit = submit(async () => {
    try {
      await session.auth.signIn(email, password);
    } catch (error_) {
      if (error_ instanceof SecondStepRequired && session.auth.steps !== undefined) {
        setPassword("");
        setStep(error_);
        return;
      }
      throw error_;
    }
    await signedIn();
  });

  if (step !== undefined && session.auth.steps !== undefined) {
    return (
      <AuthCard
        title={step.step === "verify" ? t("second-step-title") : t("second-step-enrol-title")}
        intro={step.step === "verify" ? t("second-step-intro") : t("second-step-enrol-intro")}
      >
        <SecondStep
          step={step}
          steps={session.auth.steps}
          onSignedIn={signedIn}
          onStartOver={() => {
            setStep(undefined);
          }}
        />
      </AuthCard>
    );
  }

  const sso = session.status.sso ?? undefined;
  return (
    <AuthCard title={t("sign-in-title")} intro={t("sign-in-intro")}>
      {sso !== undefined && (
        <div className="vl-auth-sso">
          <a
            className="vl-filled-button"
            href={`${sso.startUrl}${sso.startUrl.includes("?") ? "&" : "?"}${new URLSearchParams({ next }).toString()}`}
          >
            {t("sign-in-sso", { name: sso.name })}
          </a>
          <p className="vl-muted vl-auth-or">{t("sign-in-or")}</p>
        </div>
      )}
      <form className="vl-auth-form" onSubmit={onSubmit} aria-busy={busy}>
        <Field
          label={t("auth-email")}
          type="email"
          autoComplete="username"
          value={email}
          onChange={setEmail}
        />
        <Field
          label={t("auth-password")}
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={setPassword}
        />
        {error === undefined ? undefined : (
          <p className="vl-field-error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("sign-in-submit")}
        </button>
      </form>
    </AuthCard>
  );
}

/** `/setup`: creates the first administrator of a new workspace. */
export function SetupView() {
  const t = useT();
  const session = useSession();
  const navigate = useNavigate();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const { busy, error, submit } = useSubmit(t("auth-error"));

  if (!session?.status.needsSetup) return <Navigate to="/" replace />;

  const onSubmit = submit(async () => {
    await session.auth.setUp({ name, email, password });
    await session.refresh();
    void navigate("/", { replace: true });
  });

  return (
    <AuthCard title={t("setup-title")} intro={t("setup-intro")}>
      <form className="vl-auth-form" onSubmit={onSubmit} aria-busy={busy}>
        <Field
          label={t("setup-name")}
          type="text"
          autoComplete="name"
          value={name}
          onChange={setName}
        />
        <Field
          label={t("auth-email")}
          type="email"
          autoComplete="username"
          value={email}
          onChange={setEmail}
        />
        <Field
          label={t("auth-password")}
          type="password"
          autoComplete="new-password"
          minLength={12}
          hint={t("setup-password-hint")}
          value={password}
          onChange={setPassword}
        />
        {error === undefined ? undefined : (
          <p className="vl-field-error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("setup-submit")}
        </button>
      </form>
    </AuthCard>
  );
}
