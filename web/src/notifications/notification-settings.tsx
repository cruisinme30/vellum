// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useState } from "react";

import { ViewTitle } from "../help/help-link";
import { useT } from "../i18n/i18n";
import { describeError } from "../views/sign-in";

import { useNotificationServices } from "./context";
import { ThisDevice } from "./device-section";
import {
  CHANNELS,
  channelsFor,
  isUrgent,
  KINDS,
  withChannel,
  type NotificationPreferences,
} from "./preferences";

const EMAIL_DELAYS = [0, 5, 10, 30, 60] as const;
const SNOOZES = [
  ["notify-snooze-1h", 60],
  ["notify-snooze-8h", 8 * 60],
  ["notify-snooze-1d", 24 * 60],
  ["notify-snooze-1w", 7 * 24 * 60],
] as const;

/** Whether a snooze is still running at `now`. */
export function snoozeActive(
  snoozedUntil: string | undefined,
  now: number,
): boolean {
  if (snoozedUntil === undefined) return false;
  const until = Date.parse(snoozedUntil);
  return Number.isFinite(until) && until > now;
}

type Load =
  | { readonly state: "loading" }
  | { readonly state: "failed"; readonly error: string }
  | {
      readonly state: "ready";
      readonly saved: NotificationPreferences;
      readonly draft: NotificationPreferences;
    };

/** `/settings/notifications`: what reaches you, where, and when. */
export function NotificationSettingsView({
  now = Date.now,
}: {
  readonly now?: () => number;
}) {
  const t = useT();
  const { store, device } = useNotificationServices();
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();

  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let live = true;
    store.load().then(
      (preferences) => {
        if (live)
          setLoad({ state: "ready", saved: preferences, draft: preferences });
      },
      (error_: unknown) => {
        if (live)
          setLoad({
            state: "failed",
            error: describeError(error_, t("auth-error")),
          });
      },
    );
    return () => {
      live = false;
    };
  }, [store, t, attempt]);

  const retry = () => {
    setLoad({ state: "loading" });
    setAttempt((count) => count + 1);
  };

  if (load.state === "loading") {
    return (
      <div className="vl-view">
        <h1>{t("notify-title")}</h1>
        <p className="vl-muted" role="status">
          {t("notify-loading")}
        </p>
      </div>
    );
  }
  if (load.state === "failed") {
    return (
      <div className="vl-view">
        <h1>{t("notify-title")}</h1>
        <p className="vl-field-error" role="alert">
          {load.error}
        </p>
        <button type="button" className="vl-filled-button" onClick={retry}>
          {t("notify-retry")}
        </button>
      </div>
    );
  }

  const { saved, draft } = load;
  const dirty = JSON.stringify(saved) !== JSON.stringify(draft);
  const edit = (next: NotificationPreferences) => {
    setStatus(undefined);
    setLoad({ state: "ready", saved, draft: next });
  };
  const save = (event: { preventDefault(): void }) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    store.save(draft).then(
      (stored) => {
        setBusy(false);
        setLoad({ state: "ready", saved: stored, draft: stored });
        setStatus(t("notify-saved"));
      },
      (error_: unknown) => {
        setBusy(false);
        setError(describeError(error_, t("auth-error")));
      },
    );
  };

  return (
    <form className="vl-view vl-notify" onSubmit={save} aria-busy={busy}>
      <ViewTitle title={t("notify-title")} help="app.notifications" />
      <p className="vl-muted">{t("notify-intro")}</p>

      <ChannelTable draft={draft} onChange={edit} />
      <EmailDelay draft={draft} onChange={edit} />
      <Snooze draft={draft} onChange={edit} now={now} />
      <Muted draft={draft} onChange={edit} />

      <div className="vl-notify-actions">
        <button
          type="submit"
          className="vl-filled-button"
          disabled={busy || !dirty}
        >
          {t("notify-save")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy || !dirty}
          onClick={() => {
            edit(saved);
          }}
        >
          {t("notify-discard")}
        </button>
        <p className="vl-muted" role="status">
          {status}
        </p>
      </div>
      {error === undefined ? undefined : (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}

      <ThisDevice device={device} />
    </form>
  );
}

interface SectionProperties {
  readonly draft: NotificationPreferences;
  readonly onChange: (next: NotificationPreferences) => void;
}

function ChannelTable({ draft, onChange }: SectionProperties) {
  const t = useT();
  return (
    <section aria-labelledby="notify-channels-heading">
      <h2 id="notify-channels-heading">{t("notify-channels")}</h2>
      <div className="vl-notify-table-wrap">
        <table className="vl-notify-table">
          <caption className="vl-muted">{t("notify-channels-caption")}</caption>
          <thead>
            <tr>
              <th scope="col">{t("notify-kind")}</th>
              {CHANNELS.map((channel) => (
                <th scope="col" key={channel}>
                  {t(`notify-channel-${channel}`)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {KINDS.map((kind) => {
              const on = channelsFor(draft, kind);
              const label = t(`notify-kind-${kind}`);
              return (
                <tr key={kind}>
                  <th scope="row">
                    {label}
                    {isUrgent(kind) ? (
                      <span className="vl-notify-urgent">
                        {t("notify-urgent")}
                      </span>
                    ) : undefined}
                  </th>
                  {CHANNELS.map((channel) => (
                    <td key={channel}>
                      <input
                        type="checkbox"
                        checked={on.includes(channel)}
                        aria-label={t("notify-toggle", {
                          channel: t(`notify-channel-${channel}`),
                          kind: label,
                        })}
                        onChange={(event) => {
                          onChange(
                            withChannel(
                              draft,
                              kind,
                              channel,
                              event.target.checked,
                            ),
                          );
                        }}
                      />
                    </td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function EmailDelay({ draft, onChange }: SectionProperties) {
  const t = useT();
  const id = useId();
  const options: readonly number[] = EMAIL_DELAYS.includes(
    draft.emailDelayMinutes as (typeof EMAIL_DELAYS)[number],
  )
    ? EMAIL_DELAYS
    : [...EMAIL_DELAYS, draft.emailDelayMinutes].toSorted((a, b) => a - b);
  return (
    <section aria-labelledby={`${id}-heading`}>
      <h2 id={`${id}-heading`}>{t("notify-email")}</h2>
      <div className="vl-auth-field">
        <label htmlFor={id}>{t("notify-email-delay")}</label>
        <select
          id={id}
          className="vl-notify-select"
          value={draft.emailDelayMinutes}
          aria-describedby={`${id}-hint`}
          onChange={(event) => {
            onChange({
              ...draft,
              emailDelayMinutes: Number(event.target.value),
            });
          }}
        >
          {options.map((minutes) => (
            <option key={minutes} value={minutes}>
              {minutes === 0
                ? t("notify-email-now")
                : t("notify-minutes", { n: minutes })}
            </option>
          ))}
        </select>
        <small id={`${id}-hint`} className="vl-muted">
          {t("notify-email-hint")}
        </small>
      </div>
    </section>
  );
}

function Snooze({
  draft,
  onChange,
  now,
}: SectionProperties & { readonly now: () => number }) {
  const t = useT();
  const active = snoozeActive(draft.snoozedUntil, now());
  return (
    <section aria-labelledby="notify-snooze-heading">
      <h2 id="notify-snooze-heading">{t("notify-snooze")}</h2>
      {active && draft.snoozedUntil !== undefined ? (
        <p>
          {t("notify-snoozed-until", {
            time: new Date(draft.snoozedUntil).toLocaleString(undefined, {
              dateStyle: "medium",
              timeStyle: "short",
            }),
          })}{" "}
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              onChange({ ...draft, snoozedUntil: undefined });
            }}
          >
            {t("notify-resume")}
          </button>
        </p>
      ) : (
        <>
          <p className="vl-muted">{t("notify-snooze-hint")}</p>
          <div
            className="vl-notify-chips"
            role="group"
            aria-label={t("notify-snooze")}
          >
            {SNOOZES.map(([label, minutes]) => (
              <button
                key={label}
                type="button"
                className="vl-text-button"
                onClick={() => {
                  onChange({
                    ...draft,
                    snoozedUntil: new Date(
                      now() + minutes * 60_000,
                    ).toISOString(),
                  });
                }}
              >
                {t(label)}
              </button>
            ))}
          </div>
        </>
      )}
    </section>
  );
}

function Muted({ draft, onChange }: SectionProperties) {
  const t = useT();
  const id = useId();
  const [subject, setSubject] = useState("");
  const add = () => {
    const value = subject.trim();
    if (value === "" || draft.muted.includes(value)) return;
    onChange({
      ...draft,
      muted: [...draft.muted, value].toSorted((a, b) => a.localeCompare(b)),
    });
    setSubject("");
  };
  return (
    <section aria-labelledby={`${id}-heading`}>
      <h2 id={`${id}-heading`}>{t("notify-muted")}</h2>
      {draft.muted.length === 0 ? (
        <p className="vl-muted">{t("notify-muted-empty")}</p>
      ) : (
        <ul className="vl-notify-muted">
          {draft.muted.map((item) => (
            <li key={item}>
              <code>{item}</code>
              <button
                type="button"
                className="vl-text-button"
                aria-label={t("notify-unmute-label", { subject: item })}
                onClick={() => {
                  onChange({
                    ...draft,
                    muted: draft.muted.filter((each) => each !== item),
                  });
                }}
              >
                {t("notify-unmute")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="vl-notify-add">
        <div className="vl-auth-field">
          <label htmlFor={id}>{t("notify-mute-subject")}</label>
          <input
            id={id}
            type="text"
            value={subject}
            maxLength={200}
            onChange={(event) => {
              setSubject(event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                add();
              }
            }}
          />
        </div>
        <button type="button" className="vl-text-button" onClick={add}>
          {t("notify-mute")}
        </button>
      </div>
    </section>
  );
}
