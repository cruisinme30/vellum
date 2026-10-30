// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState, type ComponentProps } from "react";

import type { Provider } from "../api/api.gen";
import { failureText } from "../data/failure";
import { ServerScreen } from "../data/server-apis";
import { useT } from "../i18n/i18n";

import { useModel } from "./answer";
import type { AssistApi, TestResult } from "./api";
import {
  forgetModel,
  keyHint,
  saveModel,
  SUGGESTED_MODEL,
  toSettings,
  type StoredModel,
} from "./settings";

import "../admin/console/console.css";
import "./assist.css";

/** `/settings/ai`: the person's own model, kept in this browser. */
export function AssistantSettingsView() {
  const t = useT();
  return (
    <ServerScreen
      api="assist"
      title={t("assist-settings-title")}
      className="vl-admin"
    >
      {(api) => <AssistantSettings api={api} />}
    </ServerScreen>
  );
}

const PROVIDERS: readonly Provider[] = ["anthropic", "openai_compatible"];

const PLACEHOLDER: Readonly<Record<Provider, string>> = {
  anthropic: "https://api.anthropic.com",
  openai_compatible: "https://api.openai.com/v1",
};

/** The form's values; `apiKey` is only what was typed now. */
const EMPTY: StoredModel = {
  provider: "anthropic",
  model: SUGGESTED_MODEL.anthropic,
  baseUrl: "",
  apiKey: "",
  remember: false,
};

function Fields({
  value,
  saved,
  onChange,
}: {
  readonly value: StoredModel;
  readonly saved: StoredModel | undefined;
  readonly onChange: (next: StoredModel) => void;
}) {
  const t = useT();
  const text = (field: "model" | "baseUrl" | "apiKey") => ({
    value: value[field],
    autoComplete: "off",
    spellCheck: false,
    onChange: (event: { target: { value: string } }) => {
      onChange({ ...value, [field]: event.target.value });
    },
  });
  const pickProvider = (next: Provider) => {
    const keepModel =
      value.model !== "" && value.model !== SUGGESTED_MODEL[value.provider];
    onChange({
      ...value,
      provider: next,
      model: keepModel ? value.model : SUGGESTED_MODEL[next],
    });
  };
  return (
    <>
      <label>
        {t("assist-provider")}
        <select
          value={value.provider}
          onChange={(event) => {
            pickProvider(event.target.value as Provider);
          }}
        >
          {PROVIDERS.map((p) => (
            <option key={p} value={p}>
              {t(`assist-provider-${p}`)}
            </option>
          ))}
        </select>
      </label>
      <label>
        {t("assist-model")}
        <input required {...text("model")} />
      </label>
      <label>
        {t("assist-api-key")}
        <input
          type="password"
          placeholder={
            saved === undefined || saved.apiKey === ""
              ? ""
              : keyHint(saved.apiKey)
          }
          aria-describedby="assist-key-help"
          {...text("apiKey")}
        />
      </label>
      <p id="assist-key-help" className="vl-muted">
        {t("assist-api-key-help")}
      </p>
      <label>
        {t("assist-base-url")}
        <input
          type="url"
          placeholder={PLACEHOLDER[value.provider]}
          {...text("baseUrl")}
        />
      </label>
      <label className="vl-assist-check">
        <input
          type="checkbox"
          checked={value.remember}
          onChange={(event) => {
            onChange({ ...value, remember: event.target.checked });
          }}
        />
        {t("assist-remember")}
      </label>
    </>
  );
}

function Outcome({ result }: { readonly result: TestResult | undefined }) {
  const t = useT();
  return (
    <>
      <div role="status">
        {result?.ok === true && (
          <p>{`${result.detail} ${t("assist-saved")}`}</p>
        )}
      </div>
      {result?.ok === false && (
        <p className="vl-field-error" role="alert">
          {result.detail}
        </p>
      )}
    </>
  );
}

/** The form; exported for tests. */
export function AssistantSettings({ api }: { readonly api: AssistApi }) {
  const t = useT();
  const { userId, stored } = useModel();
  const [value, setValue] = useState<StoredModel>(() => ({
    ...(stored ?? EMPTY),
    apiKey: "",
  }));
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<TestResult>();

  if (userId === undefined) {
    return <p>{t("needs-server")}</p>;
  }
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    // An empty key field keeps the saved key.
    const typed = value.apiKey.trim();
    const next: StoredModel = {
      ...value,
      apiKey: typed === "" ? (stored?.apiKey ?? "") : typed,
    };
    setBusy(true);
    setResult(undefined);
    api
      .test(toSettings(next))
      .then(
        (tested) => {
          if (tested.ok) {
            saveModel(userId, next);
            setValue({ ...value, apiKey: "" });
          }
          setResult(tested);
        },
        (error: unknown) => {
          setResult({ ok: false, detail: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <>
      <p className="vl-muted">{t("assist-settings-intro")}</p>
      <form className="vl-admin-form" onSubmit={submit}>
        <Fields value={value} saved={stored} onChange={setValue} />
        <div className="vl-admin-actions">
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {busy ? t("assist-testing") : t("assist-save")}
          </button>
          {stored !== undefined && (
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                forgetModel(userId);
                setValue({ ...value, apiKey: "" });
                setResult(undefined);
              }}
            >
              {t("assist-forget")}
            </button>
          )}
        </div>
        <Outcome result={result} />
      </form>
    </>
  );
}
