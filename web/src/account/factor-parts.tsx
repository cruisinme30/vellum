// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import { useT } from "../i18n/i18n";

import "./account.css";

import type { TotpSetup } from "./api";
import { QrCode } from "./qr-code";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

/**
 * A one-time code field: digits, filled in by the phone's keyboard or a
 * password manager.
 */
export function CodeField({
  id,
  label,
  value,
  onChange,
  recovery = false,
}: {
  readonly id: string;
  readonly label: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
  /** Recovery codes are letters and digits, not six digits. */
  readonly recovery?: boolean;
}) {
  return (
    <>
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        className="vl-account-code"
        required
        value={value}
        autoComplete={recovery ? "off" : "one-time-code"}
        inputMode={recovery ? "text" : "numeric"}
        pattern={recovery ? undefined : "[0-9 ]{6,7}"}
        maxLength={recovery ? 20 : 7}
        spellCheck={false}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      />
    </>
  );
}

/**
 * Setting up an authenticator app: scan or type the secret, then enter a code.
 */
export function TotpSetupPanel({
  setup,
  busy,
  error,
  onConfirm,
  onCancel,
}: {
  readonly setup: TotpSetup;
  readonly busy: boolean;
  readonly error: string | undefined;
  readonly onConfirm: (code: string) => void;
  readonly onCancel?: () => void;
}) {
  const t = useT();
  const id = useId();
  const [code, setCode] = useState("");
  const submit: Submit = (event) => {
    event.preventDefault();
    onConfirm(code);
  };
  return (
    <form
      className="vl-admin-form vl-account-totp"
      onSubmit={submit}
      aria-busy={busy}
    >
      <ol className="vl-account-steps">
        <li>{t("factors-totp-scan")}</li>
        <li>
          <QrCode rows={setup.qr} label={t("factors-totp-qr")} />
          <p className="vl-muted">
            {t("factors-totp-manual")}{" "}
            <code className="vl-account-secret">{setup.secret}</code>
          </p>
          <p>
            <a href={setup.uri}>{t("factors-totp-open")}</a>
          </p>
        </li>
        <li>
          <CodeField
            id={`${id}-code`}
            label={t("factors-totp-code")}
            value={code}
            onChange={setCode}
          />
        </li>
      </ol>
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
          {t("factors-totp-confirm")}
        </button>
      </div>
    </form>
  );
}

/** New recovery codes, shown once, with a way to keep them. */
export function RecoveryCodesPanel({
  codes,
  onDone,
}: {
  readonly codes: readonly string[];
  readonly onDone: () => void;
}) {
  const t = useT();
  const id = useId();
  const [copied, setCopied] = useState(false);
  const text = `${codes.join("\n")}\n`;
  return (
    <div className="vl-admin-secret" role="region" aria-labelledby={id}>
      <p id={id}>
        <strong>{t("factors-recovery-title")}</strong>{" "}
        {t("factors-recovery-hint")}
      </p>
      <ul className="vl-account-recovery" data-testid="recovery-codes">
        {codes.map((code) => (
          <li key={code}>
            <code>{code}</code>
          </li>
        ))}
      </ul>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            navigator.clipboard.writeText(text).then(
              () => {
                setCopied(true);
              },
              () => {
                setCopied(false);
              },
            );
          }}
        >
          {copied ? t("tokens-copied") : t("factors-recovery-copy")}
        </button>
        <a
          className="vl-text-button"
          download="vellum-recovery-codes.txt"
          href={`data:text/plain;charset=utf-8,${encodeURIComponent(text)}`}
        >
          {t("factors-recovery-download")}
        </a>
        <button type="button" className="vl-filled-button" onClick={onDone}>
          {t("factors-recovery-done")}
        </button>
      </div>
    </div>
  );
}
