// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Work planning over `/api/v1/work` (R-WK-11, 12, 13, 15, 16; D223-D225):
 * projects, milestones, health updates, initiatives, templates, issue
 * creation with every field, insights, intake forms ("Asks") and the
 * waiting triage suggestion. Types come from the server's generated client.
 */

import type {
  BriefOut,
  CreatedIssue,
  FormOut,
  FormsOut,
  InitiativeOut,
  InitiativesOut,
  InsightsOut,
  IssueGroups,
  IssueOut,
  ProjectOut,
  ProjectsOut,
  TeamOut,
  TeamsOut,
  TemplateOut,
  TemplatesOut,
  UpdateOut,
  UpdatesOut,
} from "../../api/api.gen";
import type { Http } from "../../api/http";

/** A new project. */
interface NewProject {
  readonly title: string;
  readonly teams: readonly string[];
  readonly lead?: string;
  readonly status?: string;
  readonly start_date?: string;
  readonly target_date?: string;
  readonly description?: string;
}

/** A new initiative. */
interface NewInitiative {
  readonly title: string;
  readonly owner?: string;
  readonly target_date?: string;
  readonly projects: readonly string[];
  readonly description?: string;
}

/** A new issue with any of the fields the create form offers. */
export interface IssueDraft {
  readonly team: string;
  readonly title: string;
  readonly description?: string;
  readonly template?: string;
  readonly priority?: number | undefined;
  readonly estimate?: string;
  readonly assignee?: string;
  readonly labels?: readonly string[];
  readonly project?: string;
  readonly milestone?: string;
  readonly due?: string;
  readonly repeat?: string;
}

/** An insights question. */
export interface InsightsQuestion {
  readonly team?: string | undefined;
  readonly measure: string;
  readonly pivot: string;
  readonly from: string;
  readonly to: string;
  readonly interval?: string | undefined;
}

/** The planning operations the web app uses. */
export interface PlanningApi {
  teams(): Promise<TeamOut[]>;
  projects(): Promise<ProjectOut[]>;
  project(id: string): Promise<ProjectOut>;
  createProject(project: NewProject): Promise<ProjectOut>;
  addMilestone(project: string, name: string, targetDate?: string): Promise<ProjectOut>;
  /** A project's issues, open and recently closed, most important first. */
  projectIssues(project: string): Promise<IssueOut[]>;
  setMilestone(issue: string, milestone: string): Promise<IssueOut>;
  /** Health updates, newest first. */
  updates(project: string): Promise<UpdateOut[]>;
  postUpdate(project: string, health: string, body: string): Promise<UpdateOut>;
  initiatives(): Promise<InitiativeOut[]>;
  createInitiative(initiative: NewInitiative): Promise<InitiativeOut>;
  templates(team: string): Promise<TemplateOut[]>;
  /** A team's open repeating issues, soonest due first (#144). */
  recurring(team: string): Promise<IssueOut[]>;
  /**
   * Sets how an issue repeats; `repeat` "" stops it. `due` is sent when
   * given ("" clears it), before the repeat is checked against it.
   */
  setRepeat(issue: string, repeat: string, due?: string): Promise<IssueOut>;
  createIssue(draft: IssueDraft): Promise<IssueOut>;
  insights(question: InsightsQuestion): Promise<InsightsOut>;
  forms(team: string): Promise<FormOut[]>;
  submitForm(team: string, slug: string, answers: Record<string, string>): Promise<IssueOut>;
  /** The issue's triage brief, with any suggestion waiting for a person. */
  brief(issue: string): Promise<BriefOut>;
  acceptSuggestion(issue: string): Promise<IssueOut>;
  dismissSuggestion(issue: string): Promise<IssueOut>;
}

const WORK = "/api/v1/work";
const segment = encodeURIComponent;
const project = (id: string) => `${WORK}/projects/${segment(id)}`;
const issue = (key: string) => `${WORK}/issues/${segment(key)}`;

/** Drops empty strings and empty lists, so the server sees only answers. */
function compact<T extends object>(value: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(value).filter(
      ([, v]) => v !== undefined && v !== "" && !(Array.isArray(v) && v.length === 0),
    ),
  ) as Partial<T>;
}

/** Planning over a Vellum server. */
export function createHttpPlanningApi(http: Http): PlanningApi {
  return {
    async teams() {
      const found = await http.get<TeamsOut>(`${WORK}/teams`);
      return found.teams;
    },
    async projects() {
      const found = await http.get<ProjectsOut>(`${WORK}/projects`);
      return found.projects;
    },
    project: (id) => http.get<ProjectOut>(project(id)),
    createProject: (draft) => http.send<ProjectOut>("POST", `${WORK}/projects`, compact(draft)),
    addMilestone: (id, name, target_date) =>
      http.send<ProjectOut>("POST", `${project(id)}/milestones`, compact({ name, target_date })),
    async projectIssues(id) {
      const filter = JSON.stringify({ field: "project", op: "is", values: [id] });
      const found = await http.get<IssueGroups>(`${WORK}/issues`, {
        filter,
        group_by: "none",
        order_by: "priority",
        show_closed: "past_month",
      });
      return found.groups.flatMap((group) => group.issues);
    },
    setMilestone: (key, milestone) => http.send<IssueOut>("PATCH", issue(key), { milestone }),
    async recurring(team) {
      const filter = JSON.stringify({ field: "repeat", op: "is_not_empty" });
      const found = await http.get<IssueGroups>(`${WORK}/issues`, {
        team,
        filter,
        group_by: "none",
        order_by: "due",
        show_closed: "none",
      });
      return found.groups.flatMap((group) => group.issues);
    },
    setRepeat: (key, repeat, due) =>
      http.send<IssueOut>("PATCH", issue(key), due === undefined ? { repeat } : { due, repeat }),
    async updates(id) {
      const found = await http.get<UpdatesOut>(`${project(id)}/updates`);
      return found.updates;
    },
    postUpdate: (id, health, body) =>
      http.send<UpdateOut>("POST", `${project(id)}/updates`, { health, body }),
    async initiatives() {
      const found = await http.get<InitiativesOut>(`${WORK}/initiatives`);
      return found.initiatives;
    },
    createInitiative: (initiative) =>
      http.send<InitiativeOut>("POST", `${WORK}/initiatives`, compact(initiative)),
    async templates(team) {
      const found = await http.get<TemplatesOut>(`${WORK}/teams/${segment(team)}/templates`);
      return found.templates;
    },
    async createIssue(draft) {
      const created = await http.send<CreatedIssue>("POST", `${WORK}/issues`, compact(draft));
      return created.issue;
    },
    insights: (q) => http.get<InsightsOut>(`${WORK}/insights`, compact(q)),
    async forms(team) {
      const found = await http.get<FormsOut>(`${WORK}/teams/${segment(team)}/forms`);
      return found.forms;
    },
    submitForm: (team, slug, answers) =>
      http.send<IssueOut>("POST", `${WORK}/teams/${segment(team)}/forms/${segment(slug)}`, {
        answers,
      }),
    brief: (key) => http.get<BriefOut>(`${issue(key)}/triage/brief`),
    acceptSuggestion: (key) =>
      http.send<IssueOut>("POST", `${issue(key)}/triage/suggestion/accept`, {}),
    dismissSuggestion: (key) => http.send<IssueOut>("DELETE", `${issue(key)}/triage/suggestion`),
  };
}

export type {
  FormOut,
  InitiativeOut,
  InsightsOut,
  IssueOut,
  ProjectOut,
  TeamOut,
  TemplateOut,
  UpdateOut,
} from "../../api/api.gen";
