// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId } from "react";
import { Link } from "react-router";

import { useSession } from "../api/session";
import { failureText } from "../data/failure";
import { Ready } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { ActivityView, DriveApi, NodeView } from "./api";
import { actorText, formatBytes, formatWhen } from "./format";
import { openPath } from "./paths";

/** One change: who did what, and when. */
function ActivityLine({ event }: { readonly event: ActivityView }) {
  const t = useT();
  const me = useSession()?.status.user?.id;
  const what = t(`drive-activity-${event.kind}`, {
    from: event.from ?? "",
    to: event.to ?? "",
  });
  return (
    <li>
      <span>
        {actorText(t, event.actor, me)} {what}
      </span>
      <small className="vl-muted">{formatWhen(event.at)}</small>
    </li>
  );
}

/**
 * A feed of changes, newest first: to one item, or to a whole space by its ID.
 */
export function ActivityFeed({
  api,
  id,
}: {
  readonly api: DriveApi;
  readonly id: string;
}) {
  const t = useT();
  const loaded = useLoaded(() => api.activity(id), [api, id], failureText);
  return (
    <Ready loaded={loaded}>
      {(events) =>
        events.length === 0 ? (
          <p className="vl-muted">{t("drive-activity-empty")}</p>
        ) : (
          <ul className="vl-drive-activity">
            {events.map((event, index) => (
              <ActivityLine
                key={`${event.at}-${String(index)}`}
                event={event}
              />
            ))}
          </ul>
        )
      }
    </Ready>
  );
}

function Backlinks({
  api,
  id,
}: {
  readonly api: DriveApi;
  readonly id: string;
}) {
  const t = useT();
  const loaded = useLoaded(() => api.backlinks(id), [api, id], failureText);
  return (
    <Ready loaded={loaded}>
      {(pages) =>
        pages.length === 0 ? (
          <p className="vl-muted">{t("drive-backlinks-empty")}</p>
        ) : (
          <ul className="vl-drive-links">
            {pages.map((page) => (
              <li key={page.id}>
                <Link to={openPath(page) ?? "#"}>{page.title}</Link>
              </li>
            ))}
          </ul>
        )
      }
    </Ready>
  );
}

/**
 * An item's facts, its activity and, for a page, the pages linking to it
 * (R-DR-7).
 */
export function DetailsPanel({
  api,
  item,
  onClose,
}: {
  readonly api: DriveApi;
  readonly item: NodeView;
  readonly onClose: () => void;
}) {
  const t = useT();
  const headingId = useId();
  return (
    <aside className="vl-drive-details" aria-labelledby={headingId}>
      <div className="vl-drive-details-head">
        <h2 id={headingId}>{item.title}</h2>
        <button type="button" className="vl-text-button" onClick={onClose}>
          {t("drive-close")}
        </button>
      </div>
      <dl className="vl-drive-facts">
        <dt>{t("drive-kind")}</dt>
        <dd>{t(`drive-kind-${item.kind}`)}</dd>
        <dt>{t("drive-location")}</dt>
        <dd>
          <code>{item.path}</code>
        </dd>
        <dt>{t("drive-size")}</dt>
        <dd>{formatBytes(item.size)}</dd>
        <dt>{t("drive-modified")}</dt>
        <dd>{formatWhen(item.modified)}</dd>
      </dl>
      <h3>{t("drive-activity")}</h3>
      <ActivityFeed api={api} id={item.id} />
      {item.kind === "page" && (
        <>
          <h3>{t("drive-backlinks")}</h3>
          <Backlinks api={api} id={item.id} />
        </>
      )}
    </aside>
  );
}
