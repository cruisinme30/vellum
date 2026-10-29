// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type ComponentProps } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { failureText } from "../data/failure";
import { Ready, ServerScreen } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useServices, useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import type { CommentOut, IssueOut, TeamOut, WorkApi } from "./api";
import { stateName } from "./issues-page";

import "./work.css";

const TIME = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/** An issue with its team, for the state names. */
interface Loaded {
  readonly issue: IssueOut;
  readonly team: TeamOut | undefined;
}

async function loadIssue(api: WorkApi, key: string): Promise<Loaded> {
  const [issue, teams] = await Promise.all([api.issue(key), api.teams()]);
  return { issue, team: teams.find((team) => team.key === issue.team) };
}

function StatePicker({
  loaded,
  onChange,
}: {
  readonly loaded: Loaded;
  readonly onChange: (state: string) => void;
}) {
  const t = useT();
  const id = useId();
  const statuses = loaded.team?.statuses ?? [];
  if (statuses.length === 0) {
    return <p>{t("issue-state", { state: loaded.issue.state })}</p>;
  }
  return (
    <div className="vl-work-filters">
      <label htmlFor={id}>{t("issue-state-label")}</label>
      <select
        id={id}
        value={loaded.issue.state}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        {statuses.map((status) => (
          <option key={status.key} value={status.key}>
            {status.name}
          </option>
        ))}
      </select>
    </div>
  );
}

function Comments({ api, issueKey }: { readonly api: WorkApi; readonly issueKey: string }) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(() => api.comments(issueKey), [api, issueKey, version], failureText);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const body = new FormData(form).get("body");
    if (typeof body !== "string" || body.trim() === "") return;
    setError(undefined);
    api.addComment(issueKey, body.trim()).then(
      () => {
        form.reset();
        setVersion((v) => v + 1);
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  return (
    <section aria-labelledby="issue-comments">
      <h2 id="issue-comments">{t("issue-comments")}</h2>
      <Ready loaded={loaded}>{(comments) => <CommentList comments={comments} />}</Ready>
      <form className="vl-work-comment" onSubmit={submit}>
        <label>
          {t("issue-comment-add")}
          <textarea name="body" rows={3} required />
        </label>
        <button type="submit" className="vl-filled-button">
          {t("issue-comment-send")}
        </button>
      </form>
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function CommentList({ comments }: { readonly comments: readonly CommentOut[] }) {
  const t = useT();
  if (comments.length === 0) return <p className="vl-muted">{t("issue-no-comments")}</p>;
  return (
    <ol className="vl-comment-list">
      {comments.map((comment) => (
        <li key={comment.id}>
          <p className="vl-muted">
            {t("issue-comment-by", {
              author: comment.author,
              when: TIME.format(new Date(comment.created_at)),
            })}
          </p>
          <p className="vl-comment-body">{comment.body}</p>
        </li>
      ))}
    </ol>
  );
}

/** The Markdown a doc started from an issue begins with: a link back and the description. */
function pageBodyFromIssue(issue: IssueOut): string {
  const description = issue.description?.trim() ?? "";
  return description === "" ? `Issue: ${issue.key}\n` : `Issue: ${issue.key}\n\n${description}\n`;
}

/**
 * "Create doc" (R-WK-7): a page in the person's first space, titled after
 * the issue, that starts with the issue's chip and description.
 */
function CreatePageFromIssue({ issue }: { readonly issue: IssueOut }) {
  const t = useT();
  const navigate = useNavigate();
  const { store } = useServices();
  const workspace = useWorkspace();
  const [busy, setBusy] = useState(false);
  const space = workspace.spaces[0];
  if (space === undefined) return;
  return (
    <button
      type="button"
      className="vl-text-button"
      disabled={busy}
      onClick={() => {
        setBusy(true);
        void store
          .createPage({
            spaceId: space.id,
            parentId: undefined,
            title: `${issue.key} ${issue.title}`,
            body: pageBodyFromIssue(issue),
          })
          .then((page) => {
            setBusy(false);
            if (page) void navigate(`/p/${page.id}`);
          });
      }}
    >
      {t("issue-create-doc")}
    </button>
  );
}

function Issue({ api, issueKey }: { readonly api: WorkApi; readonly issueKey: string }) {
  const t = useT();
  const [version, setVersion] = useState(0);
  const [error, setError] = useState<string>();
  const loaded = useLoaded(() => loadIssue(api, issueKey), [api, issueKey, version], failureText);
  const setState = (state: string) => {
    setError(undefined);
    api.setState(issueKey, state).then(
      () => {
        setVersion((v) => v + 1);
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
  };
  return (
    <Ready loaded={loaded}>
      {(found) => (
        <>
          <p className="vl-muted">
            <Link to={`/issues?team=${encodeURIComponent(found.issue.team)}`}>
              {found.team?.name ?? found.issue.team}
            </Link>{" "}
            · {found.issue.key} · {stateName(found.team, found.issue.state)}
          </p>
          <h1>{found.issue.title}</h1>
          <StatePicker loaded={found} onChange={setState} />
          <CreatePageFromIssue issue={found.issue} />
          {error !== undefined && (
            <p className="vl-field-error" role="alert">
              {error}
            </p>
          )}
          {found.issue.description != undefined && found.issue.description !== "" ? (
            <p className="vl-issue-description">{found.issue.description}</p>
          ) : (
            <p className="vl-muted">{t("issue-no-description")}</p>
          )}
          <Comments api={api} issueKey={found.issue.key} />
        </>
      )}
    </Ready>
  );
}

/** One issue: its state, description and comments. */
export function IssuePage() {
  const { issueKey = "" } = useParams();
  return (
    <ServerScreen api="work" className="vl-work">
      {(api) => <Issue api={api} issueKey={issueKey} />}
    </ServerScreen>
  );
}
