// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useState, type ComponentProps } from "react";

import { failureText } from "../data/failure";
import { useT } from "../i18n/i18n";
import type { DirectoryApi } from "../people/directory";
import { PeoplePicker } from "../people/people-picker";
import { Icon } from "../shell/icon";

import type {
  CreatedLink,
  GrantRole,
  GrantView,
  SharingApi,
  SharingView,
} from "./api";
import { DATE, endOfDay, today } from "./dates";
import { Links } from "./share-links";

import "./sharing.css";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

const ROLES: readonly GrantRole[] = [
  "viewer",
  "commenter",
  "editor",
  "manager",
];

/** Props of {@link SharePanel}. */
export interface SharePanelProperties {
  readonly api: SharingApi;
  readonly itemId: string;
  readonly directory: DirectoryApi | undefined;
  readonly onClose: () => void;
}

/**
 * The share dialog (#95): add people and groups with a role, change or
 * remove their access, and make, copy and revoke share links. The server
 * decides who may share; someone who may not sees why.
 */
export function SharePanel({
  api,
  itemId,
  directory,
  onClose,
}: SharePanelProperties) {
  const t = useT();
  const headingId = useId();
  const [state, setState] = useState<SharingView>();
  const [error, setError] = useState<string>();
  const [created, setCreated] = useState<CreatedLink>();

  useEffect(() => {
    let current = true;
    api.state(itemId).then(
      (found) => {
        if (current) setState(found);
      },
      (error_: unknown) => {
        if (current) setError(failureText(error_));
      },
    );
    return () => {
      current = false;
    };
  }, [api, itemId]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  /** Runs a change, keeps the new state, and shows a failure. */
  const run = async (change: () => Promise<SharingView | undefined>) => {
    setError(undefined);
    try {
      const next = await change();
      setState(next ?? (await api.state(itemId)));
      return true;
    } catch (error_) {
      setError(failureText(error_));
      return false;
    }
  };

  return (
    <div
      className="vl-share-panel"
      role="dialog"
      aria-modal="false"
      aria-labelledby={headingId}
    >
      <header className="vl-share-head">
        <h2 id={headingId}>
          {state
            ? t("share-title", { title: state.item.title })
            : t("share-title-plain")}
        </h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("share-close")}
          title={t("share-close")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </header>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      {state === undefined && error === undefined && (
        <p className="vl-muted" role="status">
          {t("loading")}
        </p>
      )}
      {state !== undefined && (
        <>
          <AddPeople
            directory={directory}
            onShare={(principals, role, until) =>
              run(async () => {
                let next: SharingView | undefined;
                for (const principal of principals) {
                  next = await api.share(itemId, principal, role, until);
                }
                return next;
              })
            }
          />
          <Grants
            grants={state.grants}
            onRole={(principal, role) =>
              run(() => api.share(itemId, principal, role))
            }
            onRemove={(principal) => run(() => api.unshare(itemId, principal))}
          />
          <Links
            state={state}
            created={created}
            onCreate={async (link) => {
              setError(undefined);
              try {
                const made = await api.createLink(itemId, link);
                setCreated(made);
                setState(await api.state(itemId));
                return true;
              } catch (error_) {
                setError(failureText(error_));
                return false;
              }
            }}
            onRevoke={(linkId) =>
              run(async () => {
                await api.revokeLink(linkId);
                if (created?.link.id === linkId) setCreated(undefined);
                return api.state(itemId);
              })
            }
          />
        </>
      )}
    </div>
  );
}

function RoleSelect({
  label,
  value,
  onChange,
  name,
}: {
  readonly label: string;
  readonly value?: GrantRole;
  readonly onChange?: (role: GrantRole) => void;
  readonly name?: string;
}) {
  const t = useT();
  return (
    <label className="vl-share-field">
      <span>{label}</span>
      <select
        name={name}
        {...(value === undefined ? { defaultValue: "viewer" } : { value })}
        onChange={(event) => onChange?.(event.target.value as GrantRole)}
      >
        {ROLES.map((role) => (
          <option key={role} value={role}>
            {t(`share-role-${role}`)}
          </option>
        ))}
      </select>
    </label>
  );
}

function AddPeople({
  directory,
  onShare,
}: {
  readonly directory: DirectoryApi | undefined;
  readonly onShare: (
    principals: string[],
    role: GrantRole,
    until?: string,
  ) => Promise<boolean>;
}) {
  const t = useT();
  const [people, setPeople] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const role = data.get("role");
    const until = data.get("until");
    if (people.length === 0 || typeof role !== "string") return;
    setBusy(true);
    void onShare(
      people,
      role as GrantRole,
      typeof until === "string" ? endOfDay(until) : undefined,
    ).then((ok) => {
      setBusy(false);
      if (ok) {
        setPeople([]);
        form.reset();
      }
    });
  };
  return (
    <section aria-label={t("share-add")}>
      <form className="vl-share-add" onSubmit={submit}>
        <PeoplePicker
          label={t("share-people")}
          value={people}
          onChange={setPeople}
          directory={directory}
          hint={t("share-people-hint")}
        />
        <div className="vl-share-row">
          <RoleSelect label={t("share-role")} name="role" />
          <label className="vl-share-field">
            <span>{t("share-until")}</span>
            <input type="date" name="until" min={today()} />
          </label>
        </div>
        <button
          type="submit"
          className="vl-filled-button"
          disabled={busy || people.length === 0}
        >
          {t("share-send")}
        </button>
      </form>
    </section>
  );
}

/** A grant's name: the handle, else the principal. */
function grantName(grant: GrantView): string {
  if (grant.handle != undefined && grant.handle !== "")
    return `@${grant.handle}`;
  return grant.principal;
}

function Grants({
  grants,
  onRole,
  onRemove,
}: {
  readonly grants: readonly GrantView[];
  readonly onRole: (principal: string, role: GrantRole) => Promise<boolean>;
  readonly onRemove: (principal: string) => Promise<boolean>;
}) {
  const t = useT();
  const headingId = useId();
  return (
    <section aria-labelledby={headingId}>
      <h3 id={headingId}>{t("share-with-access")}</h3>
      <p className="vl-muted">{t("share-inherited-note")}</p>
      {grants.length === 0 ? (
        <p className="vl-muted">{t("share-no-grants")}</p>
      ) : (
        <ul className="vl-share-list">
          {grants.map((grant) => {
            const name = grantName(grant);
            return (
              <li key={`${grant.principal}-${grant.role}`}>
                <span className="vl-share-name">
                  {grant.principal.startsWith("group:")
                    ? t("share-group", { name })
                    : name}
                  {grant.expires_at != undefined && (
                    <small className="vl-muted">
                      {t("share-until-date", {
                        date: DATE.format(new Date(grant.expires_at)),
                      })}
                    </small>
                  )}
                </span>
                {grant.role === "owner" ? (
                  <span className="vl-muted">{t("share-role-owner")}</span>
                ) : (
                  <>
                    <RoleSelect
                      label={t("share-role-of", { name })}
                      value={grant.role}
                      onChange={(role) => {
                        void onRole(grant.principal, role);
                      }}
                    />
                    <button
                      type="button"
                      className="vl-text-button"
                      aria-label={t("share-remove", { name })}
                      onClick={() => {
                        void onRemove(grant.principal);
                      }}
                    >
                      {t("share-remove-short")}
                    </button>
                  </>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
