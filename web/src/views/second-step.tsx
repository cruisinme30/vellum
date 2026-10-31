// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import {
  CodeField,
  RecoveryCodesPanel,
  TotpSetupPanel,
} from "../account/factor-parts";
import {
  createPasskey,
  passkeysSupported,
  signWithPasskey,
  type CreationOptionsJson,
  type RequestOptionsJson,
} from "../account/webauthn";
import type { TotpSetup } from "../api/api.gen";
import type { SecondStepRequired, SignInSteps } from "../api/auth";
import { useT } from "../i18n/i18n";

import { describeError } from "./sign-in";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

/** Runs one request at a time, keeping its error. */
function useWork(fallback: string) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = (work: () => Promise<void>) => {
    setBusy(true);
    setError(undefined);
    work().then(
      () => {
        setBusy(false);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(
          error_ instanceof DOMException
            ? fallback
            : describeError(error_, fallback),
        );
      },
    );
  };
  return { busy, error, run, setError };
}

function ErrorLine({ error }: { readonly error: string | undefined }) {
  return error === undefined ? undefined : (
    <p className="vl-field-error" role="alert">
      {error}
    </p>
  );
}

/** A code from the app, or a recovery code, or a passkey. */
function Verify({
  step,
  steps,
  onSignedIn,
}: {
  readonly step: SecondStepRequired;
  readonly steps: SignInSteps;
  readonly onSignedIn: () => Promise<void>;
}) {
  const t = useT();
  const id = useId();
  const hasApp = step.methods.includes("totp");
  const hasPasskey = step.methods.includes("passkey") && passkeysSupported();
  const [recovery, setRecovery] = useState(!hasApp && !hasPasskey);
  const [code, setCode] = useState("");
  const { busy, error, run } = useWork(t("second-step-failed"));

  const submit: Submit = (event) => {
    event.preventDefault();
    run(async () => {
      await steps.verify(step.ticket, { code });
      await onSignedIn();
    });
  };
  const withPasskey = () => {
    run(async () => {
      const options = (await steps.passkeyOptions(
        step.ticket,
      )) as RequestOptionsJson;
      const passkey = await signWithPasskey(options);
      await steps.verify(step.ticket, { passkey });
      await onSignedIn();
    });
  };
  return (
    <>
      {hasPasskey && (
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy}
          onClick={withPasskey}
        >
          {t("second-step-passkey")}
        </button>
      )}
      {(hasApp || recovery) && (
        <form className="vl-auth-form" onSubmit={submit} aria-busy={busy}>
          <div className="vl-auth-field">
            <CodeField
              id={`${id}-code`}
              label={
                recovery
                  ? t("second-step-recovery-code")
                  : t("second-step-app-code")
              }
              value={code}
              onChange={setCode}
              recovery={recovery}
            />
          </div>
          <ErrorLine error={error} />
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("second-step-verify")}
          </button>
        </form>
      )}
      {!(hasApp || recovery) && <ErrorLine error={error} />}
      {step.methods.includes("recovery") && (
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            setRecovery(!recovery);
            setCode("");
          }}
        >
          {recovery && hasApp
            ? t("second-step-use-app")
            : t("second-step-use-recovery")}
        </button>
      )}
    </>
  );
}

/** The workspace requires a factor: add one, keep the recovery codes, go on. */
function Enrol({
  step,
  steps,
  onSignedIn,
}: {
  readonly step: SecondStepRequired;
  readonly steps: SignInSteps;
  readonly onSignedIn: () => Promise<void>;
}) {
  const t = useT();
  const id = useId();
  const [setup, setSetup] = useState<TotpSetup>();
  const [codes, setCodes] = useState<readonly string[]>();
  const [name, setName] = useState("");
  const { busy, error, run } = useWork(t("second-step-failed"));

  if (codes !== undefined) {
    return (
      <RecoveryCodesPanel
        codes={codes}
        onDone={() => {
          run(onSignedIn);
        }}
      />
    );
  }
  if (setup !== undefined) {
    return (
      <TotpSetupPanel
        setup={setup}
        busy={busy}
        error={error}
        onCancel={() => {
          setSetup(undefined);
        }}
        onConfirm={(code) => {
          run(async () => {
            const done = await steps.enrol(step.ticket, { code });
            setCodes(done.recoveryCodes);
          });
        }}
      />
    );
  }
  const addPasskey: Submit = (event) => {
    event.preventDefault();
    run(async () => {
      const options = (await steps.passkeyOptions(
        step.ticket,
      )) as CreationOptionsJson;
      const credential = await createPasskey(options);
      const done = await steps.enrol(step.ticket, {
        passkey: { name, credential },
      });
      setCodes(done.recoveryCodes);
    });
  };
  return (
    <>
      <button
        type="button"
        className="vl-filled-button"
        disabled={busy}
        onClick={() => {
          run(async () => {
            setSetup(await steps.authenticator(step.ticket));
          });
        }}
      >
        {t("second-step-enrol-app")}
      </button>
      {passkeysSupported() && (
        <form className="vl-auth-form" onSubmit={addPasskey} aria-busy={busy}>
          <div className="vl-auth-field">
            <label htmlFor={`${id}-name`}>{t("factors-passkey-name")}</label>
            <input
              id={`${id}-name`}
              required
              maxLength={60}
              value={name}
              placeholder={t("factors-passkey-name-placeholder")}
              onChange={(event) => {
                setName(event.target.value);
              }}
            />
          </div>
          <button type="submit" className="vl-text-button" disabled={busy}>
            {t("second-step-enrol-passkey")}
          </button>
        </form>
      )}
      <ErrorLine error={error} />
    </>
  );
}

/** The second step of a sign-in: verify a factor, or add the first one. */
export function SecondStep({
  step,
  steps,
  onSignedIn,
  onStartOver,
}: {
  readonly step: SecondStepRequired;
  readonly steps: SignInSteps;
  readonly onSignedIn: () => Promise<void>;
  readonly onStartOver: () => void;
}) {
  const t = useT();
  return (
    <div className="vl-second-step">
      {step.step === "verify" ? (
        <Verify step={step} steps={steps} onSignedIn={onSignedIn} />
      ) : (
        <Enrol step={step} steps={steps} onSignedIn={onSignedIn} />
      )}
      <button type="button" className="vl-text-button" onClick={onStartOver}>
        {t("second-step-start-over")}
      </button>
    </div>
  );
}
