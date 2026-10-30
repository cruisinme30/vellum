// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { failureText } from "../../data/failure";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import {
  formText,
  type AdminConsoleApi,
  type GroupMembersView,
  type GroupView,
  type UserView,
} from "./api";
import { AdminScreen } from "./context";
import type { Submit } from "./space-panel";

type Notice = { readonly ok: boolean; readonly message: string } | undefined;

/** One group's name, description and members, opened from the list. */
function GroupPanel({
  api,
  group,
  users,
  onChanged,
  onNotice,
}: {
  readonly api: AdminConsoleApi;
  readonly group: GroupView;
  readonly users: readonly UserView[];
  readonly onChanged: (deleted: boolean) => void;
  readonly onNotice: (notice?: Notice) => void;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [changed, setChanged] = useState<GroupMembersView>();
  const loaded = useLoaded(
    () => api.groupMembers(group.id),
    [api, group.id],
    failureText,
  );
  const view = changed?.group.id === group.id ? changed : undefined;
  const members =
    view?.members ?? (loaded.state === "ready" ? loaded.value.members : []);

  const run = <T,>(
    task: () => Promise<T>,
    done: string,
    after: (value: T) => void,
  ) => {
    setBusy(true);
    onNotice();
    task()
      .then(
        (value) => {
          after(value);
          onNotice({ ok: true, message: done });
        },
        (error: unknown) => {
          onNotice({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };

  const save: Submit = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    run(
      () =>
        api.updateGroup(group.id, {
          name: formText(data, "name"),
          description: formText(data, "description"),
        }),
      t("admin-groups-saved"),
      () => {
        onChanged(false);
      },
    );
  };
  const add: Submit = (event) => {
    event.preventDefault();
    const user = formText(new FormData(event.currentTarget), "user");
    if (user === "") return;
    run(
      () => api.changeGroupMembers(group.id, { add: [user] }),
      t("admin-groups-added"),
      setChanged,
    );
  };
  const outsiders = users.filter(
    (u) => !u.disabled && !members.some((m) => m.id === u.id),
  );

  return (
    <section
      className="vl-admin-panel"
      aria-label={t("admin-groups-panel", { name: group.name })}
    >
      <h2>{group.name}</h2>
      <form className="vl-admin-form" onSubmit={save} key={group.id}>
        <label>
          {t("admin-groups-name")}
          <input
            name="name"
            required
            maxLength={100}
            defaultValue={group.name}
          />
        </label>
        <label>
          {t("admin-groups-description")}
          <input
            name="description"
            maxLength={500}
            defaultValue={group.description}
          />
        </label>
        <div className="vl-admin-actions">
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("admin-save")}
          </button>
        </div>
      </form>
      <h3>{t("admin-groups-members")}</h3>
      {loaded.state === "failed" && view === undefined && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
        </p>
      )}
      {members.length === 0 && loaded.state === "ready" ? (
        <p className="vl-muted">{t("admin-groups-no-members")}</p>
      ) : (
        <ul className="vl-admin-members">
          {members.map((member) => (
            <li key={member.id}>
              <span>
                {member.name} <span className="vl-muted">@{member.handle}</span>
              </span>
              <button
                type="button"
                className="vl-text-button"
                disabled={busy}
                aria-label={t("admin-groups-remove-label", {
                  name: member.name,
                })}
                onClick={() => {
                  run(
                    () =>
                      api.changeGroupMembers(group.id, { remove: [member.id] }),
                    t("admin-groups-removed", { name: member.name }),
                    setChanged,
                  );
                }}
              >
                {t("admin-spaces-remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {outsiders.length > 0 && (
        <form
          className="vl-admin-filters"
          aria-labelledby={`${id}-add`}
          onSubmit={add}
        >
          <label>
            <span id={`${id}-add`}>{t("admin-groups-add")}</span>
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
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("admin-groups-add")}
          </button>
        </form>
      )}
      <div className="vl-admin-actions">
        {confirmDelete ? (
          <div
            className="vl-admin-row-actions"
            role="group"
            aria-label={t("admin-groups-delete-confirm", { name: group.name })}
          >
            <span>
              {t("admin-groups-delete-confirm", { name: group.name })}
            </span>
            <button
              type="button"
              className="vl-filled-button"
              disabled={busy}
              onClick={() => {
                run(
                  () => api.deleteGroup(group.id),
                  t("admin-groups-deleted", { name: group.name }),
                  () => {
                    onChanged(true);
                  },
                );
              }}
            >
              {t("admin-groups-delete")}
            </button>
            <button
              type="button"
              className="vl-text-button"
              onClick={() => {
                setConfirmDelete(false);
              }}
            >
              {t("admin-users-keep")}
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setConfirmDelete(true);
            }}
          >
            {t("admin-groups-delete")}
          </button>
        )}
      </div>
    </section>
  );
}

function CreateGroupForm({
  api,
  onCreated,
  onNotice,
}: {
  readonly api: AdminConsoleApi;
  readonly onCreated: (group: GroupView) => void;
  readonly onNotice: (notice?: Notice) => void;
}) {
  const t = useT();
  const id = useId();
  const [busy, setBusy] = useState(false);
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    setBusy(true);
    onNotice();
    api
      .createGroup(formText(data, "name"), formText(data, "description"))
      .then(
        (group) => {
          form.reset();
          onCreated(group);
          onNotice({
            ok: true,
            message: t("admin-groups-created", { name: group.name }),
          });
        },
        (error: unknown) => {
          onNotice({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <form
      className="vl-admin-form"
      aria-labelledby={`${id}-title`}
      onSubmit={submit}
    >
      <h2 id={`${id}-title`}>{t("admin-groups-create")}</h2>
      <label>
        {t("admin-groups-name")}
        <input name="name" required maxLength={100} />
      </label>
      <label>
        {t("admin-groups-description")}
        <input name="description" maxLength={500} />
      </label>
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("admin-groups-create")}
        </button>
      </div>
    </form>
  );
}

function Groups({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [selected, setSelected] = useState<string>();
  const [notice, setNotice] = useState<Notice>();
  const loaded = useLoaded(
    async () => {
      const [groups, users] = await Promise.all([api.groups(), api.users()]);
      return { groups, users };
    },
    [api, reload],
    failureText,
  );
  const current =
    loaded.state === "ready"
      ? loaded.value.groups.find((g) => g.id === selected)
      : undefined;
  return (
    <>
      <div role="status">{notice?.ok === true && <p>{notice.message}</p>}</div>
      {notice?.ok === false && (
        <p className="vl-field-error" role="alert">
          {notice.message}
        </p>
      )}
      {loaded.state === "loading" && (
        <p className="vl-muted">{t("admin-loading")}</p>
      )}
      {loaded.state === "failed" && (
        <p className="vl-field-error" role="alert">
          {loaded.message}
        </p>
      )}
      {loaded.state === "ready" && (
        <>
          {loaded.value.groups.length === 0 ? (
            <p>{t("admin-groups-empty")}</p>
          ) : (
            <div className="vl-admin-scroll">
              <table className="vl-admin-table">
                <caption>
                  {t("admin-groups-caption", {
                    count: loaded.value.groups.length,
                  })}
                </caption>
                <thead>
                  <tr>
                    <th scope="col">{t("admin-groups-name")}</th>
                    <th scope="col">{t("admin-groups-description")}</th>
                    <th scope="col">{t("admin-users-actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {loaded.value.groups.map((group) => (
                    <tr
                      key={group.id}
                      aria-current={group.id === selected || undefined}
                    >
                      <th scope="row">{group.name}</th>
                      <td>{group.description}</td>
                      <td>
                        <button
                          type="button"
                          className="vl-text-button"
                          aria-label={t("admin-spaces-manage-label", {
                            name: group.name,
                          })}
                          aria-expanded={group.id === selected}
                          onClick={() => {
                            setSelected(
                              group.id === selected ? undefined : group.id,
                            );
                          }}
                        >
                          {t("admin-spaces-manage")}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {current !== undefined && (
            <GroupPanel
              api={api}
              group={current}
              users={loaded.value.users}
              onNotice={setNotice}
              onChanged={(deleted) => {
                if (deleted) setSelected(undefined);
                setReload((n) => n + 1);
              }}
            />
          )}
          <CreateGroupForm
            api={api}
            onNotice={setNotice}
            onCreated={(group) => {
              setSelected(group.id);
              setReload((n) => n + 1);
            }}
          />
        </>
      )}
    </>
  );
}

/** Admin → Groups: named groups of people to share with at once. */
export function GroupsPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-groups")} intro={t("admin-groups-intro")}>
      {(api) => <Groups api={api} />}
    </AdminScreen>
  );
}
