// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { useSession } from "../../api/session";
import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import { formText, type AdminConsoleApi, type UserChange, type UserView } from "./api";
import { AdminScreen } from "./context";
import { SignInHelp } from "./sign-in-help";
import type { Submit } from "./space-panel";

const DAY = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });

/** The form's new account; an empty password makes a single sign-on account. */
export function newUserFromForm(data: FormData) {
  const password = formText(data, "password");
  return {
    name: formText(data, "name"),
    email: formText(data, "email"),
    admin: data.get("admin") === "on",
    ...(password === "" ? {} : { password }),
  };
}

function CreateUserForm({
  api,
  onCreated,
}: {
  readonly api: AdminConsoleApi;
  readonly onCreated: (user: UserView) => void;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    setBusy(true);
    setError(undefined);
    api
      .createUser(newUserFromForm(new FormData(form)))
      .then(
        (user) => {
          form.reset();
          onCreated(user);
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
    <form className="vl-admin-form" aria-labelledby={`${id}-title`} onSubmit={submit}>
      <h2 id={`${id}-title`}>{t("admin-users-add")}</h2>
      <label>
        {t("admin-users-name")}
        <input name="name" required autoComplete="off" />
      </label>
      <label>
        {t("admin-users-email")}
        <input name="email" type="email" required autoComplete="off" />
      </label>
      <label>
        {t("admin-users-password")}
        <input
          name="password"
          type="password"
          minLength={12}
          autoComplete="new-password"
          aria-describedby={`${id}-password-hint`}
        />
        <small id={`${id}-password-hint`} className="vl-muted">
          {t("admin-users-password-hint")}
        </small>
      </label>
      <label className="vl-admin-toggle">
        <input name="admin" type="checkbox" />
        {t("admin-users-make-admin")}
      </label>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {busy ? t("admin-saving") : t("admin-users-create")}
        </button>
      </div>
    </form>
  );
}

interface RowProperties {
  readonly user: UserView;
  readonly self: boolean;
  readonly busy: boolean;
  readonly onChange: (change: UserChange) => void;
  /** Sign-in help (issue #523); absent without the sign-in API. */
  readonly onSignInHelp?: ((help: "reset-factors" | "sign-out") => void) | undefined;
}

/** Blocking ends every session, so it asks once more first. */
function ConfirmBlock({
  user,
  busy,
  onChange,
  onKeep,
}: Omit<RowProperties, "self"> & { readonly onKeep: () => void }) {
  const t = useT();
  const question = t("admin-users-block-confirm", { name: user.name });
  return (
    <div className="vl-admin-row-actions" role="group" aria-label={question}>
      <span>{question}</span>
      <button
        type="button"
        className="vl-filled-button"
        disabled={busy}
        onClick={() => {
          onKeep();
          onChange({ disabled: true });
        }}
      >
        {t("admin-users-block")}
      </button>
      <button type="button" className="vl-text-button" onClick={onKeep}>
        {t("admin-users-keep")}
      </button>
    </div>
  );
}

/** What an admin can do to another account. */
function RowActions({ user, self, busy, onChange, onSignInHelp }: RowProperties) {
  const t = useT();
  const [confirmBlock, setConfirmBlock] = useState(false);
  if (self) return <span className="vl-muted">{t("admin-users-self-note")}</span>;
  if (confirmBlock) {
    return (
      <ConfirmBlock
        user={user}
        busy={busy}
        onChange={onChange}
        onKeep={() => {
          setConfirmBlock(false);
        }}
      />
    );
  }
  return (
    <div className="vl-admin-row-actions">
      <button
        type="button"
        className="vl-text-button"
        disabled={busy}
        aria-label={t(user.admin ? "admin-users-demote-label" : "admin-users-promote-label", {
          name: user.name,
        })}
        onClick={() => {
          onChange({ admin: !user.admin });
        }}
      >
        {user.admin ? t("admin-users-demote") : t("admin-users-promote")}
      </button>
      <button
        type="button"
        className="vl-text-button"
        disabled={busy}
        aria-label={t(user.disabled ? "admin-users-unblock-label" : "admin-users-block-label", {
          name: user.name,
        })}
        onClick={() => {
          if (user.disabled) {
            onChange({ disabled: false });
          } else {
            setConfirmBlock(true);
          }
        }}
      >
        {user.disabled ? t("admin-users-unblock") : t("admin-users-block")}
      </button>
      {onSignInHelp !== undefined && !user.disabled && (
        <SignInHelp user={user} busy={busy} onSignInHelp={onSignInHelp} />
      )}
    </div>
  );
}

/** One account's row: who, what they may do, and the admin's actions. */
function UserRow(properties: RowProperties) {
  const t = useT();
  const { user, self } = properties;
  return (
    <tr>
      <th scope="row">
        {user.name}
        {self && <span className="vl-muted"> {t("admin-users-you")}</span>}
        <div className="vl-muted">@{user.handle}</div>
      </th>
      <td>{user.email}</td>
      <td>{user.admin ? t("admin-users-role-admin") : t("admin-users-role-member")}</td>
      <td>
        {user.disabled ? (
          <span className="vl-admin-badge vl-admin-badge-off">{t("admin-users-blocked")}</span>
        ) : (
          t("admin-users-active")
        )}
      </td>
      <td>
        {user.hasPassword ? t("admin-users-by-password") : t("admin-users-by-sso")}
        {user.secondFactor === true && <div className="vl-muted">{t("admin-users-with-2fa")}</div>}
      </td>
      <td>{DAY.format(new Date(user.createdAt))}</td>
      <td>
        <RowActions {...properties} />
      </td>
    </tr>
  );
}

function Users({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const session = useSession();
  const me = session?.status.user?.id;
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState<string>();
  const [notice, setNotice] = useState<{ ok: boolean; message: string }>();
  const loaded = useLoaded(() => api.users(), [api, reload], failureText);

  // A change answers with the account as it now is, so the table keeps its
  // place; only a new account reloads it.
  const [changed, setChanged] = useState<Readonly<Record<string, UserView>>>({});
  const change = (user: UserView, next: UserChange) => {
    setBusy(user.id);
    setNotice(undefined);
    api
      .updateUser(user.id, next)
      .then(
        (updated) => {
          setChanged((before) => ({ ...before, [updated.id]: updated }));
          setNotice({ ok: true, message: t("admin-users-updated", { name: updated.name }) });
        },
        (error: unknown) => {
          setNotice({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(undefined);
      });
  };
  const signIn = api.signIn;
  const help =
    signIn === undefined
      ? undefined
      : (user: UserView, kind: "reset-factors" | "sign-out") => {
          setBusy(user.id);
          setNotice(undefined);
          const work =
            kind === "sign-out"
              ? signIn.signOutEverywhere(user.id).then((sessions) => {
                  setNotice({
                    ok: true,
                    message: t("admin-users-signed-out", { name: user.name, count: sessions }),
                  });
                })
              : signIn.resetFactors(user.id).then((updated) => {
                  setChanged((before) => ({ ...before, [updated.id]: updated }));
                  setNotice({ ok: true, message: t("admin-users-2fa-reset", { name: user.name }) });
                });
          work
            .catch((error: unknown) => {
              setNotice({ ok: false, message: failureText(error) });
            })
            .finally(() => {
              setBusy(undefined);
            });
        };
  const users =
    loaded.state === "ready" ? loaded.value.map((user) => changed[user.id] ?? user) : [];

  return (
    <>
      <div role="status">{notice?.ok === true && <p>{notice.message}</p>}</div>
      {notice?.ok === false && (
        <p className="vl-field-error" role="alert">
          {notice.message}
        </p>
      )}
      {loaded.state === "loading" && <p className="vl-muted">{t("admin-loading")}</p>}
      {loaded.state === "failed" && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
        </p>
      )}
      {loaded.state === "ready" && (
        <div className="vl-admin-scroll">
          <table className="vl-admin-table">
            <caption>{t("admin-users-caption", { count: users.length })}</caption>
            <thead>
              <tr>
                <th scope="col">{t("admin-users-name")}</th>
                <th scope="col">{t("admin-users-email")}</th>
                <th scope="col">{t("admin-users-role")}</th>
                <th scope="col">{t("admin-users-status")}</th>
                <th scope="col">{t("admin-users-sign-in")}</th>
                <th scope="col">{t("admin-users-created")}</th>
                <th scope="col">{t("admin-users-actions")}</th>
              </tr>
            </thead>
            <tbody>
              {users.map((user) => (
                <UserRow
                  key={user.id}
                  user={user}
                  self={user.id === me}
                  busy={busy === user.id}
                  onChange={(next) => {
                    change(user, next);
                  }}
                  onSignInHelp={
                    help === undefined
                      ? undefined
                      : (kind) => {
                          help(user, kind);
                        }
                  }
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
      <CreateUserForm
        api={api}
        onCreated={(user) => {
          setNotice({ ok: true, message: t("admin-users-created-notice", { name: user.name }) });
          setChanged({});
          setReload((n) => n + 1);
        }}
      />
    </>
  );
}

/** Admin → Users: every account, who administers, who is blocked, and new accounts. */
export function UsersPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-users")} intro={t("admin-users-intro")}>
      {(api) => <Users api={api} />}
    </AdminScreen>
  );
}
