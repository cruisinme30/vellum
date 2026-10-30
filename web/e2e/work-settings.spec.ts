// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { signedIn } from "./support/server-session";

test.use({
  viewport: { width: 1366, height: 768 },
  hasTouch: false,
  isMobile: false,
  serviceWorkers: "block",
});
// eslint-disable-next-line no-empty-pattern -- Playwright needs the fixtures object
test.beforeEach(({}, testInfo) => {
  test.skip(!testInfo.project.name.endsWith("-laptop"), "runs at laptop size only");
});

interface Status {
  key: string;
  name: string;
  category: string;
}

interface File {
  key: string;
  name: string;
  members?: string[];
  workflow: { statuses: Status[]; default: string };
  [field: string]: unknown;
}

/** One empty board column per status after triage. */
function columns(team: File | undefined) {
  return (team?.workflow.statuses ?? [])
    .filter((status) => status.category !== "triage")
    .map((status) => ({ key: status.key, issues: [] }));
}

/** A Work server that keeps teams in memory, as `vellum serve` keeps them in files. */
async function fakeWork(page: Page) {
  const teams = new Map<string, File>();
  const outOf = (file: File) => ({
    id: `id-${file.key}`,
    key: file.key,
    name: file.name,
    timezone: "UTC",
    members: file.members ?? [],
    statuses: file.workflow.statuses,
    default_status: file.workflow.default,
    triage: true,
    cycles: file["cycles"] !== undefined,
    labels: [],
    templates: [],
    archived: file["archived"] === true,
  });
  const standard = (key: string, name: string, members: string[]): File => ({
    id: `id-${key}`,
    key,
    name,
    members,
    last_number: 0,
    workflow: {
      default: "todo",
      statuses: [
        { key: "triage", name: "Triage", category: "triage" },
        { key: "backlog", name: "Backlog", category: "backlog" },
        { key: "todo", name: "Todo", category: "unstarted" },
        { key: "in-progress", name: "In Progress", category: "started" },
        { key: "done", name: "Done", category: "completed" },
        { key: "canceled", name: "Canceled", category: "canceled" },
        { key: "duplicate", name: "Duplicate", category: "duplicate" },
      ],
    },
    triage: true,
    sla: { rules: [] },
  });
  teams.set("ENG", standard("ENG", "Engineering", ["mara"]));
  const create = (body: Record<string, unknown>) => {
    const key = String(body["key"]);
    const members = (body["members"] as string[] | undefined) ?? [];
    const file = standard(key, String(body["name"]), members);
    teams.set(key, file);
    return outOf(file);
  };
  /** The team list; archived teams only when asked for (D303). */
  const list = (url: URL) => {
    const all = url.searchParams.get("include_archived") === "true";
    const listed = [...teams.values()].filter((file) => all || file["archived"] !== true);
    return { teams: listed.map((file) => outOf(file)) };
  };
  /** Archives or restores a team, for `POST /teams/KEY/archive|restore`. */
  const archive = (method: string, path: string) => {
    const match = /^\/teams\/([A-Z0-9]+)\/(archive|restore)$/.exec(path);
    const file = match === null ? undefined : teams.get(match[1] ?? "");
    if (method !== "POST" || match === null || file === undefined) return;
    file["archived"] = match[2] === "archive";
    return outOf(file);
  };
  /** Saves a team's settings file, for `PUT /teams/KEY`. */
  const put = (method: string, path: string, body: Record<string, unknown> | null) => {
    if (method !== "PUT" || !/^\/teams\/[A-Z0-9]+$/.test(path)) return;
    // The server parses the YAML text; the screen sends JSON, which is YAML.
    const file = JSON.parse(String(body?.["yaml"])) as File;
    teams.set(file.key, file);
    return outOf(file);
  };
  /** The answer for one request, or `undefined` for "not found". */
  const answer = (method: string, url: URL, body: Record<string, unknown> | null): unknown => {
    const path = url.pathname.replace("/api/v1/work", "");
    if (path === "/teams") return method === "POST" ? create(body ?? {}) : list(url);
    const changed = archive(method, path) ?? put(method, path, body);
    if (changed !== undefined) return changed;
    const settings = /^\/teams\/([A-Z0-9]+)\/settings$/.exec(path);
    if (settings !== null) return { settings: teams.get(settings[1] ?? ""), yaml: "" };
    if (path === "/issues")
      return { groups: columns(teams.get(url.searchParams.get("team") ?? "")) };
    return path === "/views" ? { views: [] } : undefined;
  };
  await page.route("**/api/v1/work/**", async (route) => {
    const request = route.request();
    const body = request.postDataJSON() as Record<string, unknown> | null;
    const json = answer(request.method(), new URL(request.url()), body);
    await (json === undefined
      ? route.fulfill({ status: 404, json: { code: "not_found", message: request.url() } })
      : route.fulfill({ json }));
  });
  return teams;
}

test("a new team gets a status, a one-week cycle and an SLA, and the board shows them", async ({
  page,
}) => {
  await signedIn(page);
  const teams = await fakeWork(page);
  await page.goto("/issues/settings");

  // Create ENG2.
  await page.getByRole("button", { name: "New team" }).click();
  const form = page.getByRole("form", { name: "New team" });
  await form.getByRole("textbox", { name: "Name" }).fill("Platform");
  await form.getByRole("textbox", { name: "Key prefix" }).fill("eng2");
  await form.getByRole("textbox", { name: "Members" }).fill("mara, dev");
  await form.getByRole("button", { name: "Create team" }).click();
  await expect(page).toHaveURL(/team=ENG2/);
  await expect(page.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Platform");

  // A status, a one-week cycle starting Tuesday, and an SLA rule.
  await page.getByRole("textbox", { name: "New Started status" }).fill("In Review");
  await page.getByRole("textbox", { name: "New Started status" }).press("Enter");
  await page.getByRole("checkbox", { name: "Run cycles" }).check();
  await page.getByRole("spinbutton", { name: "Length in weeks" }).fill("1");
  await page.getByRole("combobox", { name: "Starts on" }).selectOption("tuesday");
  await page.getByRole("button", { name: "Add SLA rule" }).click();
  await page.getByRole("spinbutton", { name: "Within hours" }).fill("48");

  const axe = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);

  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Settings saved." })).toBeVisible();
  const saved = teams.get("ENG2");
  expect(saved?.workflow.statuses.map((status) => status.key)).toContain("in-review");
  expect(saved?.["cycles"]).toMatchObject({ duration_weeks: 1, start_day: "tuesday" });
  expect(saved?.["sla"]).toMatchObject({ rules: [{ within_hours: 48 }] });

  // The board has a column for the new status.
  await page.goto("/issues?team=ENG2&layout=board");
  await expect(page.getByRole("region", { name: "In Review" })).toBeVisible();
});

test("an admin archives a team after confirming, and restores it", async ({ page }) => {
  await signedIn(page);
  const teams = await fakeWork(page);
  await page.goto("/issues/settings");
  await expect(page.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Engineering");

  await page.getByRole("button", { name: "Archive Engineering" }).click();
  const ask = page.getByRole("group", { name: "Archive Engineering?" });
  await expect(ask).toBeVisible();
  const axe = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);
  await ask.getByRole("button", { name: "Archive team" }).click();

  await expect(page.getByText("Engineering is archived.")).toBeVisible();
  await expect(page).toHaveURL(/team=ENG/);
  expect(teams.get("ENG")?.["archived"]).toBe(true);
  await expect(page.getByRole("combobox", { name: "Team" })).toHaveValue("ENG");
  await expect(page.getByRole("option", { name: "Engineering (archived)" })).toBeAttached();

  await page.getByRole("button", { name: "Restore team" }).click();
  await expect(page.getByRole("button", { name: "Archive Engineering" })).toBeVisible();
  expect(teams.get("ENG")?.["archived"]).toBe(false);
});
