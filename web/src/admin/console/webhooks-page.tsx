// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";

import { AdminScreen } from "./context";
import type {
  HookInput,
  HookList,
  HookView,
  WebhooksApi,
} from "./webhooks-api";
import {
  Deliveries,
  HookForm,
  Outcome,
  SecretOnce,
  Status,
  when,
  type Notice,
} from "./webhooks-parts";

/** One hook: what it gets, its health and what can be done to it. */
function HookCard({
  api,
  hook,
  eventTypes,
  onChanged,
  onSecret,
}: {
  readonly api: WebhooksApi;
  readonly hook: HookView;
  readonly eventTypes: readonly string[];
  readonly onChanged: (notice?: Notice) => void;
  readonly onSecret: (secret: string) => void;
}) {
  const t = useT();
  const [mode, setMode] = useState<"view" | "edit" | "log">("view");
  const [busy, setBusy] = useState(false);
  const quiet: Notice | undefined = undefined;
  const act = (task: Promise<Notice | undefined>) => {
    setBusy(true);
    task
      .then(
        (notice) => {
          onChanged(notice);
        },
        (error: unknown) => {
          onChanged({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  const input: HookInput = {
    url: hook.url,
    description: hook.description,
    event_types: hook.event_types,
    teams: hook.teams,
  };
  return (
    <li className="vl-admin-hook">
      <div className="vl-admin-hook-head">
        <div className="vl-admin-hook-what">
          <strong className="vl-admin-hook-url">{hook.url}</strong>
          {hook.description !== "" && (
            <span className="vl-muted">{hook.description}</span>
          )}
        </div>
        <Status hook={hook} />
      </div>
      <dl className="vl-admin-state">
        <div>
          <dt>{t("admin-webhooks-events")}</dt>
          <dd>{hook.event_types.join(", ")}</dd>
        </div>
        <div>
          <dt>{t("admin-webhooks-teams")}</dt>
          <dd>
            {hook.teams.length === 0
              ? t("admin-webhooks-all-teams")
              : hook.teams.join(", ")}
          </dd>
        </div>
        <div>
          <dt>{t("admin-webhooks-last")}</dt>
          <dd>
            {hook.last_delivery == undefined ? (
              t("admin-webhooks-never")
            ) : (
              <>
                {when(hook.last_delivery.at)} ·{" "}
                <Outcome delivery={hook.last_delivery} />
              </>
            )}
            {hook.queued > 0 && (
              <small className="vl-muted">
                {" "}
                {t("admin-webhooks-queued-count", { count: hook.queued })}
              </small>
            )}
          </dd>
        </div>
        <div>
          <dt>{t("admin-webhooks-secret")}</dt>
          <dd>
            <code>whsec_…{hook.secret_hint}</code>
            {hook.rotating && (
              <small className="vl-muted">
                {" "}
                {t("admin-webhooks-rotating")}
              </small>
            )}
          </dd>
        </div>
      </dl>
      <div className="vl-admin-actions">
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            act(
              api.test(hook.id).then((delivery) => ({
                ok: delivery.outcome === "delivered",
                message: t("admin-webhooks-tested", {
                  result: t(`admin-webhooks-outcome-${delivery.outcome}`),
                  detail: String(delivery.status ?? delivery.error ?? ""),
                }),
              })),
            );
          }}
        >
          {t("admin-webhooks-test")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={mode === "log"}
          onClick={() => {
            setMode(mode === "log" ? "view" : "log");
          }}
        >
          {t("admin-webhooks-deliveries")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={mode === "edit"}
          onClick={() => {
            setMode(mode === "edit" ? "view" : "edit");
          }}
        >
          {t("admin-webhooks-edit")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            act(api.update(hook.id, input, !hook.enabled).then(() => quiet));
          }}
        >
          {hook.enabled
            ? t("admin-webhooks-turn-off")
            : t("admin-webhooks-turn-on")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            if (!globalThis.confirm(t("admin-webhooks-rotate-confirm"))) return;
            act(
              api.rotate(hook.id).then((rotated) => {
                onSecret(rotated.secret);
                return quiet;
              }),
            );
          }}
        >
          {t("admin-webhooks-rotate")}
        </button>
        <button
          type="button"
          className="vl-text-button vl-danger"
          disabled={busy}
          onClick={() => {
            if (
              !globalThis.confirm(
                t("admin-webhooks-delete-confirm", { url: hook.url }),
              )
            )
              return;
            act(
              api.remove(hook.id).then(() => ({
                ok: true,
                message: t("admin-webhooks-deleted"),
              })),
            );
          }}
        >
          {t("admin-webhooks-delete")}
        </button>
      </div>
      {mode === "edit" && (
        <HookForm
          eventTypes={eventTypes}
          initial={hook}
          submitLabel={t("admin-save")}
          onCancel={() => {
            setMode("view");
          }}
          onSubmit={async (next) => {
            await api.update(hook.id, next, hook.enabled);
            setMode("view");
            onChanged({ ok: true, message: t("admin-webhooks-saved") });
          }}
        />
      )}
      {mode === "log" && <Deliveries api={api} hook={hook} />}
    </li>
  );
}

function Hooks({ api }: { readonly api: WebhooksApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [notice, setNotice] = useState<Notice>();
  const [secret, setSecret] = useState<string>();
  const loaded = useLoaded(() => api.list(), [api, reload], failureText);
  // Keep showing the list while it reloads after a change, so open
  // panels and focus stay where they were.
  const [shown, setShown] = useState<HookList>();
  if (loaded.state === "ready" && loaded.value !== shown)
    setShown(loaded.value);
  const changed = (next?: Notice) => {
    setNotice(next);
    setReload((n) => n + 1);
  };
  const current = loaded.state === "ready" ? loaded.value : shown;
  if (current === undefined) {
    return loaded.state === "failed" ? (
      <p className="vl-field-error" role="alert">
        {loaded.message}
        <CopyDiagnostic message={loaded.message} />
      </p>
    ) : (
      <p className="vl-muted">{t("admin-loading")}</p>
    );
  }
  const { hooks, event_types: eventTypes } = current;
  return (
    <>
      {secret !== undefined && (
        <SecretOnce
          secret={secret}
          onDone={() => {
            setSecret(undefined);
          }}
        />
      )}
      <div role="status">{notice?.ok === true && <p>{notice.message}</p>}</div>
      {notice?.ok === false && (
        <p className="vl-field-error" role="alert">
          {notice.message}
          <CopyDiagnostic message={notice.message} />
        </p>
      )}
      {hooks.length === 0 ? (
        <p>{t("admin-webhooks-empty")}</p>
      ) : (
        <ul className="vl-admin-hooks">
          {hooks.map((hook) => (
            <HookCard
              key={hook.id}
              api={api}
              hook={hook}
              eventTypes={eventTypes}
              onChanged={changed}
              onSecret={setSecret}
            />
          ))}
        </ul>
      )}
      <h2>{t("admin-webhooks-add")}</h2>
      <HookForm
        eventTypes={eventTypes}
        submitLabel={t("admin-webhooks-create")}
        onSubmit={async (input) => {
          const created = await api.create(input);
          setSecret(created.secret);
          changed({ ok: true, message: t("admin-webhooks-created") });
        }}
      />
    </>
  );
}

/** Admin → Webhooks: signed event deliveries to other systems. */
export function WebhooksPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-webhooks")} intro={t("admin-webhooks-intro")}>
      {(api) =>
        api.webhooks === undefined ? (
          <p>{t("admin-needs-server")}</p>
        ) : (
          <Hooks api={api.webhooks} />
        )
      }
    </AdminScreen>
  );
}
