// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import "../admin/console/console.css";
import "./account.css";

import type { AccountApi, FactorAdded, FactorsView, TotpSetup } from "./api";
import { RecoveryCodesPanel, TotpSetupPanel } from "./factor-parts";
import { createPasskey, passkeysSupported } from "./webauthn";

const DAY = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });

type Notice = { readonly ok: boolean; readonly message: string } | undefined;

function AddPasskeyForm({
  api,
  onAdded,
  onFailed,
}: {
  readonly api: AccountApi;
  readonly onAdded: (added: FactorAdded) => void;
  readonly onFailed: (message: string) => void;
}) {
  const t = useT();
  const id = useId();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    setBusy(true);
    api
      .startPasskey()
      .then(async ({ ticket, options }) => {
        const credential = await createPasskey(options);
        return api.addPasskey(ticket, name, credential);
      })
      .then(
        (added) => {
          setName("");
          onAdded(added);
        },
        (error: unknown) => {
          onFailed(failureText(error));
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  if (!passkeysSupported()) return <p className="vl-muted">{t("factors-passkey-unsupported")}</p>;
  return (
    <form className="vl-admin-form" onSubmit={submit} aria-busy={busy}>
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
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("factors-passkey-add")}
        </button>
      </div>
    </form>
  );
}

type Run = <T>(work: Promise<T>, done: (value: T) => void) => void;

/** The authenticator app: add it with a scan and a code, or remove it. */
function TotpSection({
  api,
  on,
  busy,
  run,
  onAdded,
  onRemoved,
}: {
  readonly api: AccountApi;
  readonly on: boolean;
  readonly busy: boolean;
  readonly run: Run;
  readonly onAdded: (result: FactorAdded) => void;
  readonly onRemoved: (next: FactorsView) => void;
}) {
  const t = useT();
  const [setup, setSetup] = useState<TotpSetup>();
  const [confirming, setConfirming] = useState(false);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string>();
  if (on) {
    return (
      <div className="vl-admin-row-actions">
        <span className="vl-admin-badge vl-account-on">{t("factors-on")}</span>
        {confirming ? (
          <>
            <span>{t("factors-totp-remove-confirm")}</span>
            <button
              type="button"
              className="vl-filled-button"
              disabled={busy}
              onClick={() => {
                setConfirming(false);
                run(api.removeTotp(), onRemoved);
              }}
            >
              {t("factors-remove")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                setConfirming(false);
              }}
            >
              {t("tokens-keep")}
            </button>
          </>
        ) : (
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            onClick={() => {
              setConfirming(true);
            }}
          >
            {t("factors-totp-remove")}
          </button>
        )}
      </div>
    );
  }
  if (setup === undefined) {
    return (
      <button
        type="button"
        className="vl-filled-button"
        disabled={busy}
        onClick={() => {
          run(api.startTotp(), setSetup);
        }}
      >
        {t("factors-totp-add")}
      </button>
    );
  }
  return (
    <TotpSetupPanel
      setup={setup}
      busy={checking}
      error={error}
      onCancel={() => {
        setSetup(undefined);
      }}
      onConfirm={(code) => {
        setChecking(true);
        setError(undefined);
        api
          .confirmTotp(code)
          .then(
            (result) => {
              setSetup(undefined);
              onAdded(result);
            },
            (error_: unknown) => {
              setError(failureText(error_));
            },
          )
          .finally(() => {
            setChecking(false);
          });
      }}
    />
  );
}

function Factors({ api, initial }: { readonly api: AccountApi; readonly initial: FactorsView }) {
  const t = useT();
  const [factors, setFactors] = useState(initial);
  const [codes, setCodes] = useState<readonly string[]>();
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice>();

  const run: Run = (work, done) => {
    setBusy(true);
    setNotice(undefined);
    work
      .then(done, (error_: unknown) => {
        setNotice({ ok: false, message: failureText(error_) });
      })
      .finally(() => {
        setBusy(false);
      });
  };
  const added = (result: FactorAdded, message: string) => {
    setFactors(result.factors);
    if (result.recoveryCodes !== null && result.recoveryCodes !== undefined) {
      setCodes(result.recoveryCodes);
    }
    setNotice({ ok: true, message });
  };

  return (
    <>
      <div role="status">{notice?.ok === true && <p>{notice.message}</p>}</div>
      {notice?.ok === false && (
        <p className="vl-field-error" role="alert">
          {notice.message}
        </p>
      )}
      {factors.required && <p className="vl-account-required">{t("factors-required")}</p>}
      {codes !== undefined && (
        <RecoveryCodesPanel
          codes={codes}
          onDone={() => {
            setCodes(undefined);
          }}
        />
      )}

      <section aria-labelledby="factors-app">
        <h2 id="factors-app">{t("factors-totp-title")}</h2>
        <p className="vl-muted">{t("factors-totp-intro")}</p>
        <TotpSection
          api={api}
          on={factors.totp}
          busy={busy}
          run={run}
          onAdded={(result) => {
            added(result, t("factors-totp-added"));
          }}
          onRemoved={(next) => {
            setFactors(next);
            setNotice({ ok: true, message: t("factors-totp-removed") });
          }}
        />
      </section>

      <section aria-labelledby="factors-passkeys">
        <h2 id="factors-passkeys">{t("factors-passkeys-title")}</h2>
        <p className="vl-muted">{t("factors-passkeys-intro")}</p>
        {factors.passkeys.length > 0 && (
          <div className="vl-admin-scroll">
            <table className="vl-admin-table">
              <caption>{t("factors-passkeys-caption", { count: factors.passkeys.length })}</caption>
              <thead>
                <tr>
                  <th scope="col">{t("tokens-name")}</th>
                  <th scope="col">{t("tokens-created")}</th>
                  <th scope="col">{t("tokens-last-used")}</th>
                  <th scope="col">{t("tokens-actions")}</th>
                </tr>
              </thead>
              <tbody>
                {factors.passkeys.map((passkey) => (
                  <tr key={passkey.id}>
                    <th scope="row">{passkey.name}</th>
                    <td>{DAY.format(new Date(passkey.createdAt))}</td>
                    <td>
                      {passkey.lastUsedAt === null || passkey.lastUsedAt === undefined
                        ? t("tokens-never-used")
                        : DAY.format(new Date(passkey.lastUsedAt))}
                    </td>
                    <td>
                      <button
                        type="button"
                        className="vl-text-button"
                        disabled={busy}
                        aria-label={t("factors-passkey-remove-label", { name: passkey.name })}
                        onClick={() => {
                          run(api.removePasskey(passkey.id), (next) => {
                            setFactors(next);
                            setNotice({
                              ok: true,
                              message: t("factors-passkey-removed", { name: passkey.name }),
                            });
                          });
                        }}
                      >
                        {t("factors-remove")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <AddPasskeyForm
          api={api}
          onAdded={(result) => {
            added(result, t("factors-passkey-added"));
          }}
          onFailed={(message) => {
            setNotice({ ok: false, message });
          }}
        />
      </section>

      {(factors.totp || factors.passkeys.length > 0) && (
        <section aria-labelledby="factors-recovery">
          <h2 id="factors-recovery">{t("factors-recovery-heading")}</h2>
          <p className="vl-muted">
            {t("factors-recovery-left", { count: factors.recoveryCodesLeft })}
          </p>
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            onClick={() => {
              run(api.newRecoveryCodes(), (fresh) => {
                setCodes(fresh);
                setFactors({ ...factors, recoveryCodesLeft: fresh.length });
              });
            }}
          >
            {t("factors-recovery-new")}
          </button>
        </section>
      )}
    </>
  );
}

function Security({ api }: { readonly api: AccountApi }) {
  const loaded = useLoaded(() => api.factors(), [api], failureText);
  return <Ready loaded={loaded}>{(factors) => <Factors api={api} initial={factors} />}</Ready>;
}

/** `/settings/security`: two-step verification with an app, passkeys and recovery codes. */
export function SecurityPage() {
  const t = useT();
  return (
    <ServerScreen api="account" title={t("factors-title")} className="vl-admin">
      {(api) => (
        <>
          <p className="vl-muted">{t("factors-intro")}</p>
          <Security api={api} />
        </>
      )}
    </ServerScreen>
  );
}
