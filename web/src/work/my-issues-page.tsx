// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * My Issues (R-WK-5): assigned, created, subscribed and recent activity.
 * Assigned issues come in focus order: urgent, SLA-bound, blocked, this
 * cycle, other active, triage, backlog, done.
 */

import { Link, useSearchParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { FocusedIssue, MyIssuesTab, WorkApi } from "./api";
import { IssueChips } from "./chips";
import { useWorkKeys, WorkNav } from "./nav";

import "./work.css";

const TABS: readonly MyIssuesTab[] = [
  "assigned",
  "created",
  "subscribed",
  "activity",
];

/** Consecutive issues with the same focus, in order. */
function byFocus(
  issues: readonly FocusedIssue[],
): { focus: string; issues: FocusedIssue[] }[] {
  const out: { focus: string; issues: FocusedIssue[] }[] = [];
  for (const each of issues) {
    const last = out.at(-1);
    if (last?.focus === each.focus) last.issues.push(each);
    else out.push({ focus: each.focus, issues: [each] });
  }
  return out;
}

function IssueRows({ issues }: { readonly issues: readonly FocusedIssue[] }) {
  return (
    <ul className="vl-issue-list">
      {issues.map(({ issue }) => (
        <li key={issue.id}>
          <Link
            to={`/issues/${encodeURIComponent(issue.key)}`}
            data-issue={issue.key}
          >
            <span className="vl-issue-key">{issue.key}</span>
            <span className="vl-issue-title">{issue.title}</span>
            <IssueChips issue={issue} />
          </Link>
        </li>
      ))}
    </ul>
  );
}

function Tab({
  api,
  tab,
}: {
  readonly api: WorkApi;
  readonly tab: MyIssuesTab;
}) {
  const t = useT();
  const loaded = useLoaded(() => api.myIssues(tab), [api, tab], failureText);
  return (
    <Ready loaded={loaded}>
      {(found) => {
        if (found.issues.length === 0)
          return <p className="vl-muted">{t(`work-mine-empty-${tab}`)}</p>;
        if (tab !== "assigned") return <IssueRows issues={found.issues} />;
        return (
          <>
            {byFocus(found.issues).map((group) => (
              <section
                key={group.focus}
                aria-labelledby={`focus-${group.focus}`}
              >
                <h2 id={`focus-${group.focus}`} className="vl-group-heading">
                  {t(`work-focus-${group.focus}`)}{" "}
                  <span className="vl-muted">{group.issues.length}</span>
                </h2>
                <IssueRows issues={group.issues} />
              </section>
            ))}
          </>
        );
      }}
    </Ready>
  );
}

function MyIssues({ api }: { readonly api: WorkApi }) {
  const t = useT();
  const [parameters, setParameters] = useSearchParams();
  const asked = parameters.get("tab");
  const tab = TABS.find((each) => each === asked) ?? "assigned";
  useWorkKeys(parameters.get("team") ?? undefined);
  return (
    <>
      <div
        className="vl-segmented"
        role="group"
        aria-label={t("work-mine-tabs")}
      >
        {TABS.map((each) => (
          <button
            key={each}
            type="button"
            aria-pressed={tab === each}
            onClick={() => {
              const next = new URLSearchParams(parameters);
              next.set("tab", each);
              setParameters(next, { replace: true });
            }}
          >
            {t(`work-mine-${each}`)}
          </button>
        ))}
      </div>
      <Tab api={api} tab={tab} />
    </>
  );
}

/** My Issues screen. */
export function MyIssuesPage() {
  const t = useT();
  return (
    <ServerScreen api="work" title={t("work-mine")} className="vl-work">
      {(api) => (
        <>
          <WorkNav />
          <MyIssues api={api} />
        </>
      )}
    </ServerScreen>
  );
}
