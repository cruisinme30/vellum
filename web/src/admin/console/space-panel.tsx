// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, type ComponentProps } from "react";

import { useT } from "../../i18n/i18n";

import {
  formText,
  type AdminConsoleApi,
  type AdminSpaceView,
  type SettingsView,
  type SpaceRoleView,
  type UserView,
} from "./api";

const ROLES: readonly SpaceRoleView[] = ["viewer", "commenter", "editor", "manager", "owner"];
const MIB = 1024 * 1024;

/** A form's submit handler. */
export type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

/**
 * The space's settings from the form: the quota in MiB (empty for no
 * limit) and trash days; the version retention is kept as it was.
 */
export function settingsFromSpaceForm(base: SettingsView, data: FormData): SettingsView {
  const quota = formText(data, "quota");
  const trash = formText(data, "trash_days");
  const next: SettingsView = {
    ...base,
    trash_days: trash === "" ? base.trash_days : Number(trash),
  };
  // No quota is no limit: the field is left out rather than sent empty.
  delete next.quota_bytes;
  if (quota !== "") next.quota_bytes = Math.round(Number(quota) * MIB);
  return next;
}

/** Runs a change and shows how it went; `fresh` reloads the whole list after. */
export type Run = (task: () => Promise<AdminSpaceView>, done: string, fresh?: boolean) => void;

/** A person's name, or their ID when the account is gone. */
export function nameOf(users: readonly UserView[], id: string | null = ""): string {
  const key = id ?? "";
  return users.find((user) => user.id === key)?.name ?? key;
}

function SettingsForm({
  api,
  space,
  busy,
  run,
}: {
  readonly api: AdminConsoleApi;
  readonly space: AdminSpaceView;
  readonly busy: boolean;
  readonly run: Run;
}) {
  const t = useT();
  const settings = space.space.settings;
  const submit: Submit = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const name = formText(data, "name");
    run(
      () =>
        api.updateSpace(space.space.id, {
          ...(name === space.space.name ? {} : { name }),
          settings: settingsFromSpaceForm(settings, data),
        }),
      t("admin-spaces-saved"),
    );
  };
  return (
    <form className="vl-admin-form" onSubmit={submit} key={space.space.id}>
      <label>
        {t("admin-spaces-name")}
        <input name="name" required defaultValue={space.space.name} />
      </label>
      <label>
        {t("admin-spaces-quota")}
        <input
          name="quota"
          type="number"
          min={1}
          step="any"
          defaultValue={
            typeof settings.quota_bytes === "number" ? String(settings.quota_bytes / MIB) : ""
          }
        />
      </label>
      <label>
        {t("admin-spaces-trash-days")}
        <input name="trash_days" type="number" min={1} defaultValue={String(settings.trash_days)} />
      </label>
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("admin-save")}
        </button>
      </div>
    </form>
  );
}

function Members({
  api,
  space,
  users,
  busy,
  run,
}: {
  readonly api: AdminConsoleApi;
  readonly space: AdminSpaceView;
  readonly users: readonly UserView[];
  readonly busy: boolean;
  readonly run: Run;
}) {
  const t = useT();
  const id = useId();
  const spaceId = space.space.id;
  if (space.space.kind === "user_drive") {
    return <p className="vl-muted">{t("admin-spaces-my-drive-note")}</p>;
  }
  const outsiders = users.filter(
    (user) => !user.disabled && !space.members.some((m) => m.user_id === user.id),
  );
  const add: Submit = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const user = formText(data, "user");
    const role = formText(data, "role") as SpaceRoleView;
    if (user === "") return;
    run(
      () => api.setMember(spaceId, user, role),
      t("admin-spaces-member-added", { name: nameOf(users, user) }),
    );
  };
  return (
    <>
      <table className="vl-admin-table">
        <caption>{t("admin-spaces-members-caption", { name: space.space.name })}</caption>
        <thead>
          <tr>
            <th scope="col">{t("admin-spaces-member")}</th>
            <th scope="col">{t("admin-spaces-role")}</th>
            <th scope="col">{t("admin-users-actions")}</th>
          </tr>
        </thead>
        <tbody>
          {space.members.map((member) => {
            const name = nameOf(users, member.user_id);
            return (
              <tr key={member.user_id}>
                <th scope="row">{name}</th>
                <td>
                  <select
                    aria-label={t("admin-spaces-role-of", { name })}
                    value={member.role}
                    disabled={busy}
                    onChange={(event) => {
                      const role = event.target.value as SpaceRoleView;
                      run(
                        () => api.setMember(spaceId, member.user_id, role),
                        t("admin-spaces-member-changed", { name }),
                      );
                    }}
                  >
                    {ROLES.map((role) => (
                      <option key={role} value={role}>
                        {t(`admin-spaces-role-${role}`)}
                      </option>
                    ))}
                  </select>
                </td>
                <td>
                  <button
                    type="button"
                    className="vl-text-button"
                    disabled={busy}
                    aria-label={t("admin-spaces-remove-label", { name })}
                    onClick={() => {
                      run(
                        () => api.setMember(spaceId, member.user_id, undefined),
                        t("admin-spaces-member-removed", { name }),
                      );
                    }}
                  >
                    {t("admin-spaces-remove")}
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {outsiders.length > 0 && (
        <form className="vl-admin-filters" aria-labelledby={`${id}-add`} onSubmit={add}>
          <h3 id={`${id}-add`} className="vl-visually-hidden">
            {t("admin-spaces-add-member")}
          </h3>
          <label>
            {t("admin-spaces-member")}
            <select name="user" required defaultValue="">
              <option value="" disabled>
                {t("admin-spaces-pick-user")}
              </option>
              {outsiders.map((user) => (
                <option key={user.id} value={user.id}>
                  {user.name} ({user.email})
                </option>
              ))}
            </select>
          </label>
          <label>
            {t("admin-spaces-role")}
            <select name="role" defaultValue="editor">
              {ROLES.map((role) => (
                <option key={role} value={role}>
                  {t(`admin-spaces-role-${role}`)}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("admin-spaces-add-member")}
          </button>
        </form>
      )}
    </>
  );
}

/** One space's settings and members, opened from the list. */
export function SpacePanel({
  api,
  space,
  users,
  busy,
  run,
}: {
  readonly api: AdminConsoleApi;
  readonly space: AdminSpaceView;
  readonly users: readonly UserView[];
  readonly busy: boolean;
  readonly run: Run;
}) {
  const t = useT();
  return (
    <section
      className="vl-admin-panel"
      aria-label={t("admin-spaces-panel", { name: space.space.name })}
    >
      <h2>{space.space.name}</h2>
      <SettingsForm api={api} space={space} busy={busy} run={run} />
      <h3>{t("admin-spaces-members")}</h3>
      <Members api={api} space={space} users={users} busy={busy} run={run} />
    </section>
  );
}
