// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { failureText } from "../../data/failure";
import { formatSize } from "../../data/format";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";

import {
  formText,
  type AdminConsoleApi,
  type AdminSpaceView,
  type UserView,
} from "./api";
import { AdminScreen } from "./context";
import { nameOf, SpacePanel, type Run, type Submit } from "./space-panel";

/**
 * A folder-path name from a display name: lowercase letters, digits and dashes.
 */
export function slugOf(name: string): string {
  return name
    .normalize("NFKD")
    .toLowerCase()
    .replaceAll(/\p{M}/gu, "")
    .split(/[^a-z0-9]+/)
    .filter((part) => part !== "")
    .join("-")
    .slice(0, 48);
}

function usage(space: AdminSpaceView, t: ReturnType<typeof useT>): string {
  const quota = space.space.settings.quota_bytes;
  return typeof quota === "number"
    ? t("admin-spaces-used-of", {
        used: formatSize(space.used_bytes),
        quota: formatSize(quota),
      })
    : formatSize(space.used_bytes);
}

function CreateSpaceForm({
  api,
  users,
  busy,
  run,
}: {
  readonly api: AdminConsoleApi;
  readonly users: readonly UserView[];
  readonly busy: boolean;
  readonly run: Run;
}) {
  const t = useT();
  const id = useId();
  const [name, setName] = useState("");
  const [slug, setSlug] = useState<string>();
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const input = {
      name: formText(data, "name"),
      slug: formText(data, "slug"),
      owner_id: formText(data, "owner"),
    };
    run(
      async () => {
        const made = await api.createSpace(input);
        form.reset();
        setName("");
        setSlug(undefined);
        return made;
      },
      t("admin-spaces-created", { name: input.name }),
      true,
    );
  };
  return (
    <form
      className="vl-admin-form"
      aria-labelledby={`${id}-title`}
      onSubmit={submit}
    >
      <h2 id={`${id}-title`}>{t("admin-spaces-create")}</h2>
      <label>
        {t("admin-spaces-name")}
        <input
          name="name"
          required
          value={name}
          onChange={(event) => {
            setName(event.target.value);
          }}
        />
      </label>
      <label>
        {t("admin-spaces-slug")}
        <input
          name="slug"
          required
          pattern="[a-z0-9]+(-[a-z0-9]+)*"
          value={slug ?? slugOf(name)}
          aria-describedby={`${id}-slug-hint`}
          onChange={(event) => {
            setSlug(event.target.value);
          }}
        />
        <small id={`${id}-slug-hint`} className="vl-muted">
          {t("admin-spaces-slug-hint")}
        </small>
      </label>
      <label>
        {t("admin-spaces-owner")}
        <select name="owner" required defaultValue="">
          <option value="" disabled>
            {t("admin-spaces-pick-user")}
          </option>
          {users
            .filter((user) => !user.disabled)
            .map((user) => (
              <option key={user.id} value={user.id}>
                {user.name} ({user.email})
              </option>
            ))}
        </select>
      </label>
      <div className="vl-admin-actions">
        <button type="submit" className="vl-filled-button" disabled={busy}>
          {t("admin-spaces-create")}
        </button>
      </div>
    </form>
  );
}

function Spaces({ api }: { readonly api: AdminConsoleApi }) {
  const t = useT();
  const [reload, setReload] = useState(0);
  const [selected, setSelected] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ ok: boolean; message: string }>();
  const loaded = useLoaded(
    async () => {
      const [spaces, users] = await Promise.all([api.spaces(), api.users()]);
      return { spaces, users };
    },
    [api, reload],
    failureText,
  );

  // A change answers with the space as it now is, so the list keeps its
  // place instead of loading again; a new space reloads the list.
  const [changed, setChanged] = useState<
    Readonly<Record<string, AdminSpaceView>>
  >({});
  const run: Run = (task, done, fresh = false) => {
    setBusy(true);
    setNotice(undefined);
    task()
      .then(
        (space) => {
          setSelected(space.space.id);
          setNotice({ ok: true, message: done });
          if (fresh) {
            setChanged({});
            setReload((n) => n + 1);
          } else {
            setChanged((before) => ({ ...before, [space.space.id]: space }));
          }
        },
        (error: unknown) => {
          setNotice({ ok: false, message: failureText(error) });
        },
      )
      .finally(() => {
        setBusy(false);
      });
  };
  const spaces =
    loaded.state === "ready"
      ? loaded.value.spaces.map((space) => changed[space.space.id] ?? space)
      : [];

  const current = spaces.find((space) => space.space.id === selected);
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
          {spaces.length === 0 ? (
            <p>{t("admin-spaces-empty")}</p>
          ) : (
            <div className="vl-admin-scroll">
              <table className="vl-admin-table">
                <caption>
                  {t("admin-spaces-caption", { count: spaces.length })}
                </caption>
                <thead>
                  <tr>
                    <th scope="col">{t("admin-spaces-name")}</th>
                    <th scope="col">{t("admin-spaces-kind")}</th>
                    <th scope="col">{t("admin-spaces-owner")}</th>
                    <th scope="col">{t("admin-spaces-members")}</th>
                    <th scope="col">{t("admin-spaces-used")}</th>
                    <th scope="col">{t("admin-users-actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {spaces.map((space) => {
                    const owner =
                      space.space.owner ??
                      space.members.find((m) => m.role === "owner")?.user_id ??
                      space.members.find((m) => m.role === "manager")?.user_id;
                    return (
                      <tr
                        key={space.space.id}
                        aria-current={space.space.id === selected || undefined}
                      >
                        <th scope="row">
                          {space.space.name}
                          <div className="vl-muted">{space.space.slug}</div>
                        </th>
                        <td>
                          {space.space.kind === "shared_drive"
                            ? t("admin-spaces-kind-shared")
                            : t("admin-spaces-kind-mine")}
                        </td>
                        <td>{nameOf(loaded.value.users, owner)}</td>
                        <td>{space.members.length}</td>
                        <td>{usage(space, t)}</td>
                        <td>
                          <button
                            type="button"
                            className="vl-text-button"
                            aria-label={t("admin-spaces-manage-label", {
                              name: space.space.name,
                            })}
                            aria-expanded={space.space.id === selected}
                            onClick={() => {
                              setSelected(
                                space.space.id === selected
                                  ? undefined
                                  : space.space.id,
                              );
                            }}
                          >
                            {t("admin-spaces-manage")}
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
          {current !== undefined && (
            <SpacePanel
              api={api}
              space={current}
              users={loaded.value.users}
              busy={busy}
              run={run}
            />
          )}
          <CreateSpaceForm
            api={api}
            users={loaded.value.users}
            busy={busy}
            run={run}
          />
        </>
      )}
    </>
  );
}

/**
 * Admin → Spaces: every shared drive and My Drive, their size, settings and
 * members.
 */
export function SpacesPage() {
  const t = useT();
  return (
    <AdminScreen title={t("admin-spaces")} intro={t("admin-spaces-intro")}>
      {(api) => <Spaces api={api} />}
    </AdminScreen>
  );
}
