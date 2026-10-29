// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Contextual help (R-HG-1, ADR 0024 D256): a `?` beside a screen's title
 * that opens the help book at the heading for that screen. Against a
 * Vellum server it opens the books built into that server at
 * `/guide/topics/<id>`, so the help matches the server's version and works
 * offline; in the demo it opens the published guide.
 */

import { useSession } from "../api/session";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { HELP_TOPICS, type HelpTopic } from "./topics.gen";

/** Where the published help books live. */
export const PUBLISHED_DOCS = "https://cruisinme30.github.io/vellum/docs/";

/** The published site's URL for a topic: `.md` pages are `.html`, and a
 * folder's `README.md` is its `index.html`. */
export function publishedUrl(topic: HelpTopic): string {
  const [path = "", anchor] = HELP_TOPICS[topic].split("#");
  const page = path.replace(/(^|\/)README\.md$/, "$1index.html").replace(/\.md$/, ".html");
  const fragment = anchor === undefined ? "" : "#" + anchor;
  return PUBLISHED_DOCS + page + fragment;
}

/** The server's URL for a topic. */
export function serverUrl(topic: HelpTopic): string {
  return `/guide/topics/${encodeURIComponent(topic)}`;
}

/** The `?` link for `topic`; `label` names what it explains. */
export function HelpLink({ topic, label }: { readonly topic: HelpTopic; readonly label: string }) {
  const t = useT();
  const session = useSession();
  const name = t("help-link", { topic: label });
  return (
    <a
      className="vl-icon-button vl-help-link"
      href={session === undefined ? publishedUrl(topic) : serverUrl(topic)}
      target="_blank"
      rel="noreferrer"
      aria-label={name}
      title={name}
    >
      <Icon name="help" size={20} />
    </a>
  );
}

/** A screen's title with its `?` link. */
export function ViewTitle({
  title,
  help,
}: {
  readonly title: string;
  readonly help?: HelpTopic | undefined;
}) {
  return (
    <div className="vl-title-row">
      <h1>{title}</h1>
      {help !== undefined && <HelpLink topic={help} label={title} />}
    </div>
  );
}
