// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useMemo, useRef, useState, type CSSProperties } from "react";

import { useT, type Translate } from "../i18n/i18n";
import { mediaQuery } from "../shell/use-media";

import { effectiveSeed, resolveTheme } from "./appearance";
import { useAppearance } from "./appearance-context";
import {
  customScheme,
  exportTheme,
  importTheme,
  themeFileName,
  TOKEN_NAMES,
  type CustomTheme,
  type ThemeFileError,
} from "./theme-file";
import {
  contrastReport,
  THEMES,
  type ContrastResult,
  type ResolvedTheme,
} from "./themes";

const BASES = THEMES.filter(
  (theme): theme is ResolvedTheme => theme !== "system",
);

/** Saves text as a file through a temporary link. */
function download(name: string, text: string) {
  const url = URL.createObjectURL(
    new Blob([text], { type: "application/json" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}

/** The words for a refused theme file. */
function importMessage(t: Translate, error: ThemeFileError): string {
  switch (error.kind) {
    case "token": {
      return t("theme-import-token", { token: error.token });
    }
    case "contrast": {
      return t("theme-import-contrast", { count: error.failures.length });
    }
    default: {
      return t(`theme-import-${error.kind}`);
    }
  }
}

const ratioText = (ratio: number) => (Math.floor(ratio * 100) / 100).toFixed(2);

/** The contrast check of the draft: every rule, failures first. */
function ContrastReport({
  results,
}: {
  readonly results: readonly ContrastResult[];
}) {
  const t = useT();
  const failures = results.filter((rule) => rule.ratio < rule.min);
  return (
    <div className="vl-contrast">
      <p
        role="status"
        className={failures.length > 0 ? "vl-field-error" : "vl-muted"}
      >
        {failures.length === 0
          ? t("contrast-pass", { count: results.length })
          : t("contrast-fail", { count: failures.length })}
      </p>
      <div className="vl-contrast-wrap">
        <table className="vl-contrast-table">
          <caption className="vl-muted">{t("contrast-caption")}</caption>
          <thead>
            <tr>
              <th scope="col">{t("contrast-pair")}</th>
              <th scope="col">{t("contrast-ratio")}</th>
              <th scope="col">{t("contrast-needs")}</th>
              <th scope="col">{t("contrast-result")}</th>
            </tr>
          </thead>
          <tbody>
            {results
              .toSorted(
                (a, b) => Number(a.ratio >= a.min) - Number(b.ratio >= b.min),
              )
              .map((rule) => {
                const ok = rule.ratio >= rule.min;
                return (
                  <tr key={`${rule.fg}/${rule.bg}`} data-ok={ok}>
                    <th scope="row">
                      <code>{rule.fg}</code> / <code>{rule.bg}</code>
                    </th>
                    <td>{ratioText(rule.ratio)}</td>
                    <td>{rule.min}</td>
                    <td>{ok ? t("contrast-ok") : t("contrast-low")}</td>
                  </tr>
                );
              })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function Preview({ draft }: { readonly draft: CustomTheme }) {
  const t = useT();
  const style = Object.fromEntries(
    Object.entries(customScheme(draft)).map(([token, color]) => [
      `--md-${token}`,
      color,
    ]),
  ) as CSSProperties;
  return (
    <div
      className="vl-theme-preview"
      style={style}
      aria-label={t("theme-preview")}
      role="group"
    >
      <p className="vl-theme-preview-title">{draft.name}</p>
      <p>{t("theme-preview-text")}</p>
      <p className="vl-theme-preview-muted">{t("theme-preview-muted")}</p>
      <span className="vl-theme-preview-button">
        {t("theme-preview-button")}
      </span>
      <p className="vl-theme-preview-callout">{t("theme-preview-callout")}</p>
    </div>
  );
}

function TokenInputs({
  draft,
  onChange,
}: {
  readonly draft: CustomTheme;
  readonly onChange: (next: CustomTheme) => void;
}) {
  const t = useT();
  const scheme = customScheme(draft);
  return (
    <details className="vl-theme-tokens">
      <summary>{t("theme-tokens")}</summary>
      <div className="vl-theme-token-grid">
        {TOKEN_NAMES.map((token) => (
          <label key={token} className="vl-theme-token">
            <input
              type="color"
              value={scheme[token] ?? "#000000"}
              onChange={(event) => {
                onChange({
                  ...draft,
                  tokens: { ...draft.tokens, [token]: event.target.value },
                });
              }}
            />
            <code>{token}</code>
          </label>
        ))}
      </div>
      <button
        type="button"
        className="vl-text-button"
        disabled={Object.keys(draft.tokens).length === 0}
        onClick={() => {
          onChange({ ...draft, tokens: {} });
        }}
      >
        {t("theme-tokens-reset")}
      </button>
    </details>
  );
}

function StartFields({
  draft,
  onChange,
}: {
  readonly draft: CustomTheme;
  readonly onChange: (next: CustomTheme) => void;
}) {
  const t = useT();
  const nameId = useId();
  const baseId = useId();
  const seedId = useId();
  return (
    <div className="vl-theme-start">
      <label htmlFor={nameId}>{t("theme-custom-name")}</label>
      <input
        id={nameId}
        maxLength={64}
        value={draft.name}
        onChange={(event) => {
          onChange({ ...draft, name: event.target.value });
        }}
      />
      <label htmlFor={baseId}>{t("theme-custom-base")}</label>
      <select
        id={baseId}
        value={draft.base}
        onChange={(event) => {
          const base =
            BASES.find((each) => each === event.target.value) ?? draft.base;
          // Overrides were picked for the old start; a new start begins clean.
          onChange({ ...draft, base, tokens: {} });
        }}
      >
        {BASES.map((base) => (
          <option key={base} value={base}>
            {t(`theme-${base}`)}
          </option>
        ))}
      </select>
      <label htmlFor={seedId}>{t("theme-seed")}</label>
      <input
        id={seedId}
        type="color"
        value={draft.seed}
        onChange={(event) => {
          onChange({ ...draft, seed: event.target.value });
        }}
      />
    </div>
  );
}

function useDraft(): [CustomTheme, (next: CustomTheme) => void] {
  const t = useT();
  const { appearance, organisation } = useAppearance();
  const [draft, setDraft] = useState<CustomTheme>(
    () =>
      appearance.custom ?? {
        name: t("theme-custom-default-name"),
        base: resolveTheme(
          appearance.theme,
          mediaQuery("(prefers-color-scheme: dark)")?.matches ?? false,
          appearance.custom,
        ),
        seed: effectiveSeed(appearance, organisation?.seed),
        tokens: {},
      },
  );
  return [draft, setDraft];
}

/**
 * The theme editor (R-TH-2): build, check, import and export a custom theme.
 */
export function ThemeEditor() {
  const t = useT();
  const { update } = useAppearance();
  const [draft, setDraft] = useDraft();
  const [message, setMessage] = useState<{ ok: boolean; text: string }>();
  const fileInput = useRef<HTMLInputElement>(null);
  const results = useMemo(() => contrastReport(customScheme(draft)), [draft]);
  const failing = results.some((rule) => rule.ratio < rule.min);
  const named = draft.name.trim() !== "";

  const change = (next: CustomTheme) => {
    setDraft(next);
    setMessage(undefined);
  };

  const readFile = async (file: File) => {
    const result = importTheme(await file.text());
    if (result.ok) {
      setDraft(result.theme);
      setMessage({
        ok: true,
        text: t("theme-import-done", { name: result.theme.name }),
      });
    } else {
      setMessage({ ok: false, text: importMessage(t, result.error) });
    }
  };

  return (
    <section
      aria-labelledby="vl-theme-editor"
      className="vl-appearance-section vl-theme-editor"
    >
      <h2 id="vl-theme-editor">{t("theme-editor-title")}</h2>
      <p className="vl-muted">{t("theme-editor-intro")}</p>
      <StartFields draft={draft} onChange={change} />
      <TokenInputs draft={draft} onChange={change} />
      <Preview draft={draft} />
      <ContrastReport results={results} />
      <div className="vl-theme-actions">
        <button
          type="button"
          className="vl-filled-button"
          disabled={failing || !named}
          onClick={() => {
            update({
              custom: { ...draft, name: draft.name.trim() },
              theme: "custom",
            });
            setMessage({ ok: true, text: t("theme-applied") });
          }}
        >
          {t("theme-apply")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={!named}
          onClick={() => {
            download(themeFileName(draft), exportTheme(draft));
          }}
        >
          {t("theme-export")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            fileInput.current?.click();
          }}
        >
          {t("theme-import")}
        </button>
        <input
          ref={fileInput}
          hidden
          type="file"
          accept="application/json,.json"
          aria-label={t("theme-import")}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file !== undefined) void readFile(file);
          }}
        />
      </div>
      <div role="status">{message?.ok === true && <p>{message.text}</p>}</div>
      {message?.ok === false && (
        <p className="vl-field-error" role="alert">
          {message.text}
        </p>
      )}
    </section>
  );
}
