// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import type { CreatedLink, LinkView, SharingView } from "./api";
import { DATE, endOfDay, today } from "./dates";

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

function linkState(link: LinkView): string {
  if (link.live) return "share-link-active";
  const expired =
    link.expires_at != undefined &&
    new Date(link.expires_at).getTime() <= Date.now();
  return expired ? "share-link-expired" : "share-link-revoked";
}

function CreatedNotice({ created }: { readonly created: CreatedLink }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const fieldId = useId();
  return (
    <div className="vl-share-created" role="status">
      <label htmlFor={fieldId}>{t("share-link-new")}</label>
      <div className="vl-share-row">
        <input
          id={fieldId}
          readOnly
          value={created.url}
          onFocus={(event) => {
            event.currentTarget.select();
          }}
        />
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            void navigator.clipboard.writeText(created.url).then(() => {
              setCopied(true);
            });
          }}
        >
          <Icon name="content_copy" />
          {t(copied ? "share-copied" : "share-copy")}
        </button>
      </div>
      <p className="vl-muted">{t("share-link-once")}</p>
    </div>
  );
}

/** Share links to a page: make, copy once, list and revoke (#95). */
export function Links({
  state,
  created,
  onCreate,
  onRevoke,
}: {
  readonly state: SharingView;
  readonly created: CreatedLink | undefined;
  readonly onCreate: (link: {
    expiresAt?: string;
    password?: string;
  }) => Promise<boolean>;
  readonly onRevoke: (linkId: string) => Promise<boolean>;
}) {
  const t = useT();
  const headingId = useId();
  if (state.item.kind !== "page") return;
  if (!state.links_allowed) {
    return (
      <section aria-labelledby={headingId}>
        <h3 id={headingId}>{t("share-links")}</h3>
        <p className="vl-muted">{t("share-links-off")}</p>
      </section>
    );
  }
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const until = data.get("until");
    const password = data.get("password");
    const expiresAt = typeof until === "string" ? endOfDay(until) : undefined;
    void onCreate({
      ...(expiresAt === undefined ? {} : { expiresAt }),
      ...(typeof password === "string" && password !== "" ? { password } : {}),
    }).then((ok) => {
      if (ok) form.reset();
    });
  };
  return (
    <section aria-labelledby={headingId}>
      <h3 id={headingId}>{t("share-links")}</h3>
      <p className="vl-muted">{t("share-links-note")}</p>
      {created && <CreatedNotice created={created} />}
      <form className="vl-share-add" onSubmit={submit}>
        <div className="vl-share-row">
          <label className="vl-share-field">
            <span>{t("share-link-expires")}</span>
            <input type="date" name="until" min={today()} />
          </label>
          <label className="vl-share-field">
            <span>{t("share-link-password")}</span>
            <input
              type="password"
              name="password"
              minLength={8}
              autoComplete="new-password"
            />
          </label>
        </div>
        <button type="submit" className="vl-filled-button">
          <Icon name="add_link" />
          {t("share-link-create")}
        </button>
      </form>
      {state.links.length > 0 && (
        <ul className="vl-share-list">
          {state.links.map((link) => (
            <li key={link.id}>
              <span className="vl-share-name">
                {t("share-link-made", {
                  date: DATE.format(new Date(link.created_at)),
                })}
                <small className="vl-muted">
                  {t(linkState(link))}
                  {link.has_password && ` · ${t("share-link-has-password")}`}
                  {link.expires_at != undefined &&
                    ` · ${t("share-until-date", { date: DATE.format(new Date(link.expires_at)) })}`}
                </small>
              </span>
              {link.live && (
                <button
                  type="button"
                  className="vl-text-button"
                  onClick={() => {
                    void onRevoke(link.id);
                  }}
                >
                  {t("share-link-revoke")}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
