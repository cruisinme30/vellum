// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
  type ComponentProps,
} from "react";

import { passkeysSupported, signWithPasskey } from "../account/webauthn";
import { useT } from "../i18n/i18n";
import { describeError } from "../views/sign-in";

import type { Http } from "./http";
import {
  needsSecondFactor,
  reauthenticate,
  reauthPasskeyOptions,
  ssoReauthUrl,
  type ReauthFactor,
  type ReauthGate,
} from "./reauth";
import { useSession } from "./session";

import "./reauth-prompt.css";

/**
 * The "confirm it's you" dialog (R-SE-7, D241). It opens when an action
 * needs a recent sign-in, takes the password again (or sends the person
 * through single sign-on), and lets the waiting action run.
 */
export function ReauthPrompt({
  gate,
  http,
}: {
  readonly gate: ReauthGate;
  readonly http: Http;
}) {
  const subscribe = useCallback(
    (listener: () => void) => gate.subscribe(listener),
    [gate],
  );
  const pending = useCallback(() => gate.pending(), [gate]);
  const open = useSyncExternalStore(subscribe, pending, pending);
  return open ? <ReauthDialog gate={gate} http={http} /> : undefined;
}

function CancelButton({ gate }: { readonly gate: ReauthGate }) {
  const t = useT();
  return (
    <button
      type="button"
      className="vl-text-button"
      onClick={() => {
        gate.settle(false);
      }}
    >
      {t("reauth-cancel")}
    </button>
  );
}

/** The password form; a wrong password says so and clears the field. */
function PasswordForm({
  gate,
  http,
  id,
}: {
  readonly gate: ReauthGate;
  readonly http: Http;
  readonly id: string;
}) {
  const t = useT();
  const input = useRef<HTMLInputElement>(null);
  const codeInput = useRef<HTMLInputElement>(null);
  const [password, setPassword] = useState("");
  // Shown once the server says the person has a second factor (D290).
  const [askCode, setAskCode] = useState(false);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    input.current?.focus();
  }, []);
  useEffect(() => {
    if (askCode) codeInput.current?.focus();
  }, [askCode]);
  const attempt = (factor: ReauthFactor | Promise<ReauthFactor>) => {
    setBusy(true);
    setError(undefined);
    Promise.resolve(factor)
      .then((proof) => reauthenticate(http, password, proof))
      .then(
        () => {
          gate.settle(true);
        },
        (error_: unknown) => {
          setBusy(false);
          if (needsSecondFactor(error_)) {
            setAskCode(true);
            return;
          }
          setCode("");
          if (!askCode) setPassword("");
          setError(
            error_ instanceof DOMException
              ? t("reauth-failed")
              : describeError(error_, t("reauth-failed")),
          );
          (askCode ? codeInput : input).current?.focus();
        },
      );
  };
  const confirm: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    attempt(askCode ? { code } : {});
  };
  const withPasskey = () => {
    attempt(
      reauthPasskeyOptions(http).then(async ({ ticket, options }) => ({
        ticket,
        passkey: await signWithPasskey(options),
      })),
    );
  };
  const errorId = `${id}-error`;
  return (
    <form className="vl-reauth-form" onSubmit={confirm}>
      <label htmlFor={`${id}-password`}>{t("auth-password")}</label>
      <input
        ref={input}
        id={`${id}-password`}
        type="password"
        autoComplete="current-password"
        required
        readOnly={askCode}
        value={password}
        aria-invalid={error === undefined || askCode ? undefined : true}
        aria-describedby={error === undefined ? undefined : errorId}
        onChange={(event) => {
          setPassword(event.target.value);
        }}
      />
      {askCode && (
        <>
          <label htmlFor={`${id}-code`}>{t("reauth-code")}</label>
          <input
            ref={codeInput}
            id={`${id}-code`}
            required
            autoComplete="one-time-code"
            spellCheck={false}
            maxLength={20}
            value={code}
            aria-invalid={error === undefined ? undefined : true}
            aria-describedby={error === undefined ? `${id}-code-hint` : errorId}
            onChange={(event) => {
              setCode(event.target.value);
            }}
          />
          <small id={`${id}-code-hint`} className="vl-muted">
            {t("reauth-code-hint")}
          </small>
        </>
      )}
      {error !== undefined && (
        <p id={errorId} className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-reauth-actions">
        <CancelButton gate={gate} />
        {askCode && passkeysSupported() && (
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            onClick={withPasskey}
          >
            {t("reauth-passkey")}
          </button>
        )}
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {busy ? t("reauth-checking") : t("reauth-confirm")}
        </button>
      </div>
    </form>
  );
}

function ReauthDialog({
  gate,
  http,
}: {
  readonly gate: ReauthGate;
  readonly http: Http;
}) {
  const t = useT();
  const session = useSession();
  const id = useId();
  const sso = session?.status.sso ?? undefined;
  // Without account details, offer the password: the server decides.
  const withPassword = session?.status.user?.hasPassword !== false;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        gate.settle(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [gate]);

  const next = `${location.pathname}${location.search}${location.hash}`;
  return (
    <div className="vl-reauth-backdrop">
      <div
        className="vl-reauth"
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        aria-describedby={`${id}-intro`}
      >
        <h2 id={`${id}-title`}>{t("reauth-title")}</h2>
        <p id={`${id}-intro`} className="vl-muted">
          {withPassword ? t("reauth-intro") : t("reauth-intro-sso")}
        </p>
        {withPassword && <PasswordForm gate={gate} http={http} id={id} />}
        {sso !== undefined && (
          <div className="vl-reauth-actions">
            {!withPassword && <CancelButton gate={gate} />}
            <a
              className="vl-text-button"
              href={ssoReauthUrl(sso.startUrl, next)}
            >
              {t("reauth-sso", { name: sso.name })}
            </a>
          </div>
        )}
      </div>
    </div>
  );
}
