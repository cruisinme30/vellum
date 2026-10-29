// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The webhooks admin's pieces: the form, the secret box and the delivery log. */

import { useId, useState, type ComponentProps } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";

import { ScrollRegion } from "./scroll-region";
import type { DeliveryView, HookInput, HookView, WebhooksApi } from "./webhooks-api";

const TIME = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/** A server time, short. */
export const when = (iso: string) => {
  const time = Date.parse(iso);
  return Number.isFinite(time) ? TIME.format(new Date(time)) : iso;
};

/** A line under the list: what happened, or what went wrong. */
export interface Notice {
  readonly ok: boolean;
  readonly message: string;
}

/** Team keys typed as `ENG, ops`: upper-cased, blanks dropped. */
export function parseTeams(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[\s,]+/u)
        .map((team) => team.trim().toUpperCase())
        .filter((team) => team !== ""),
    ),
  ];
}

function startingValues(initial: HookView | undefined) {
  return initial === undefined
    ? { url: "", description: "", events: ["issue"], teams: "" }
    : {
        url: initial.url,
        description: initial.description,
        events: initial.event_types,
        teams: initial.teams.join(", "),
      };
}

/** The add/edit form. */
export function HookForm({
  eventTypes,
  initial,
  submitLabel,
  onSubmit,
  onCancel,
}: {
  readonly eventTypes: readonly string[];
  readonly initial?: HookView;
  readonly submitLabel: string;
  readonly onSubmit: (input: HookInput) => Promise<void>;
  readonly onCancel?: () => void;
}) {
  const t = useT();
  const id = useId();
  const start = startingValues(initial);
  const [url, setUrl] = useState(start.url);
  const [description, setDescription] = useState(start.description);
  const [events, setEvents] = useState<readonly string[]>(start.events);
  const [teams, setTeams] = useState(start.teams);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    onSubmit({ url: url.trim(), description, event_types: [...events], teams: parseTeams(teams) })
      .then(
        () => {
          if (initial === undefined) {
            setUrl("");
            setDescription("");
            setTeams("");
          }
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
    <form className="vl-admin-form" onSubmit={submit} aria-busy={busy}>
      <label>
        {t("admin-webhooks-url")}
        <input
          type="url"
          required
          value={url}
          placeholder="https://hooks.northstar.dev/vellum"
          aria-describedby={`${id}-url-hint`}
          onChange={(event) => {
            setUrl(event.target.value);
          }}
        />
        <small id={`${id}-url-hint`} className="vl-muted">
          {t("admin-webhooks-url-hint")}
        </small>
      </label>
      <label>
        {t("admin-webhooks-description")}
        <input
          value={description}
          maxLength={200}
          onChange={(event) => {
            setDescription(event.target.value);
          }}
        />
      </label>
      <fieldset className="vl-admin-fieldset">
        <legend>{t("admin-webhooks-events")}</legend>
        <div className="vl-admin-checks">
          {eventTypes.map((type) => (
            <label key={type} className="vl-admin-check">
              <input
                type="checkbox"
                checked={events.includes(type)}
                onChange={(event) => {
                  setEvents(
                    event.target.checked
                      ? [...events, type]
                      : events.filter((each) => each !== type),
                  );
                }}
              />
              <code>{type}</code>
            </label>
          ))}
        </div>
      </fieldset>
      <label>
        {t("admin-webhooks-teams")}
        <input
          value={teams}
          placeholder="ENG, OPS"
          aria-describedby={`${id}-teams-hint`}
          onChange={(event) => {
            setTeams(event.target.value);
          }}
        />
        <small id={`${id}-teams-hint`} className="vl-muted">
          {t("admin-webhooks-teams-hint")}
        </small>
      </label>
      <div className="vl-admin-actions">
        <button
          type="submit"
          className="vl-filled-button"
          disabled={busy || events.length === 0 || url.trim() === ""}
        >
          {busy ? t("admin-saving") : submitLabel}
        </button>
        {onCancel !== undefined && (
          <button type="button" className="vl-text-button" onClick={onCancel}>
            {t("admin-webhooks-cancel")}
          </button>
        )}
      </div>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
          <CopyDiagnostic message={error} />
        </p>
      )}
    </form>
  );
}

/** A secret shown once, after creating a hook or rotating its secret. */
export function SecretOnce({
  secret,
  onDone,
}: {
  readonly secret: string;
  readonly onDone: () => void;
}) {
  const t = useT();
  const id = useId();
  return (
    <div className="vl-admin-secret" role="region" aria-labelledby={id}>
      <p id={id}>
        <strong>{t("admin-webhooks-secret-title")}</strong> {t("admin-webhooks-secret-hint")}
      </p>
      <code className="vl-admin-secret-value">{secret}</code>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            void navigator.clipboard.writeText(secret);
          }}
        >
          {t("admin-webhooks-copy")}
        </button>
        <button type="button" className="vl-filled-button" onClick={onDone}>
          {t("admin-webhooks-secret-done")}
        </button>
      </div>
    </div>
  );
}

export function Outcome({ delivery }: { readonly delivery: DeliveryView }) {
  const t = useT();
  const ok = delivery.outcome === "delivered";
  const detail = delivery.status ?? delivery.error ?? "";
  return (
    <span className={ok ? "vl-admin-ok" : "vl-admin-bad"}>
      {t(`admin-webhooks-outcome-${delivery.outcome}`)}
      {detail === "" ? "" : ` (${String(detail)})`}
    </span>
  );
}

/** A hook's recent attempts, with "send again". */
export function Deliveries({ api, hook }: { readonly api: WebhooksApi; readonly hook: HookView }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [notice, setNotice] = useState<Notice>();
  const loaded = useLoaded(() => api.deliveries(hook.id), [api, hook.id, reload], failureText);
  if (loaded.state === "loading") return <p className="vl-muted">{t("admin-loading")}</p>;
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
        <CopyDiagnostic message={loaded.message} />
      </p>
    );
  }
  if (loaded.value.length === 0)
    return <p className="vl-muted">{t("admin-webhooks-no-deliveries")}</p>;
  return (
    <ScrollRegion label={t("admin-webhooks-deliveries")}>
      {notice !== undefined && (
        <p className={notice.ok ? "" : "vl-field-error"} role={notice.ok ? "status" : "alert"}>
          {notice.message}
        </p>
      )}
      <table className="vl-admin-table">
        <caption>{t("admin-webhooks-deliveries-caption", { url: hook.url })}</caption>
        <thead>
          <tr>
            <th scope="col">{t("admin-webhooks-when")}</th>
            <th scope="col">{t("admin-webhooks-event")}</th>
            <th scope="col">{t("admin-webhooks-attempt")}</th>
            <th scope="col">{t("admin-webhooks-result")}</th>
            <th scope="col">
              <span className="vl-visually-hidden">{t("admin-webhooks-actions")}</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {loaded.value.map((delivery) => (
            <tr key={`${delivery.delivery_id}:${String(delivery.attempt)}:${delivery.at}`}>
              <td>{when(delivery.at)}</td>
              <td>
                <code>{delivery.event}</code>
              </td>
              <td>{delivery.attempt}</td>
              <td>
                <Outcome delivery={delivery} />
                {delivery.next_attempt != undefined && (
                  <small className="vl-muted">
                    {" "}
                    {t("admin-webhooks-next", { when: when(delivery.next_attempt) })}
                  </small>
                )}
              </td>
              <td>
                {delivery.event !== "ping" && (
                  <button
                    type="button"
                    className="vl-text-button"
                    onClick={() => {
                      api.redeliver(hook.id, delivery.delivery_id).then(
                        () => {
                          setNotice({ ok: true, message: t("admin-webhooks-queued") });
                          setReload((n) => n + 1);
                        },
                        (error: unknown) => {
                          setNotice({ ok: false, message: failureText(error) });
                        },
                      );
                    }}
                  >
                    {t("admin-webhooks-redeliver")}
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </ScrollRegion>
  );
}

export function Status({ hook }: { readonly hook: HookView }) {
  const t = useT();
  if (hook.enabled) {
    return <span className="vl-admin-ok">{t("admin-webhooks-on")}</span>;
  }
  if (hook.paused_reason != undefined) {
    return (
      <span className="vl-admin-bad">
        {t("admin-webhooks-paused", { reason: hook.paused_reason })}
      </span>
    );
  }
  return <span className="vl-muted">{t("admin-webhooks-off")}</span>;
}
