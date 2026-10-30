// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

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

const CSV = [
  "TYPE,CONTENT,DESCRIPTION,PRIORITY,INDENT,AUTHOR,RESPONSIBLE,DATE,DATE_LANG,TIMEZONE",
  "task,Renew the certificates,,1,1,,,,en,",
  "task,Book the venue,,4,1,,,,en,",
].join("\n");

const task = (id: string, title: string) => ({
  id,
  title,
  assignees: [],
  done: false,
  fields: { labels: [], remind: [] },
  kind: "checklist",
  list: "Ops",
  overdue: false,
  subtasks: [],
});

test("imports a Todoist CSV from My Work settings and shows the tasks", async ({ page }) => {
  await signedIn(page);
  let imported = false;
  await page.route("**/api/v1/my-work/connections", (route) =>
    route.fulfill({
      json: {
        caldav_url: "https://vellum.northstar.dev/dav/calendars/priya/",
        email: true,
        slack: false,
        slack_linked: false,
      },
    }),
  );
  await page.route("**/api/v1/tasks/import", async (route) => {
    const body = route.request().postDataJSON() as { from: string; project?: string };
    expect(body).toMatchObject({ from: "todoist", project: "Ops" });
    imported = true;
    await route.fulfill({ json: { lists: [{ doc: "01M64LIST", title: "Ops", added: 2 }] } });
  });
  await page.route("**/api/v1/my-work", (route) =>
    route.fulfill({
      json: {
        open: imported ? 2 : 0,
        today: "2026-10-29",
        sections: imported
          ? [
              {
                bucket: "inbox",
                tasks: [
                  task("01M64LIST.b1", "Renew the certificates"),
                  task("01M64LIST.b2", "Book the venue"),
                ],
              },
            ]
          : [],
      },
    }),
  );
  await page.route("**/api/v1/my-work/settings", (route) =>
    route.fulfill({
      json: {
        effective_time_zone: "UTC",
        day_starts_at: "09:00",
      },
    }),
  );

  await page.goto("/my-work/settings");
  await expect(page.getByRole("heading", { level: 1, name: "My Work settings" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "CalDAV account URL" })).toHaveValue(
    "https://vellum.northstar.dev/dav/calendars/priya/",
  );
  const axe = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);

  await page.getByRole("combobox", { name: "From" }).selectOption("todoist");
  await page.getByLabel("Export file").setInputFiles({
    name: "ops.csv",
    mimeType: "text/csv",
    buffer: Buffer.from(CSV),
  });
  await expect(page.getByText("ops.csv holds 2 tasks.")).toBeVisible();
  await page.getByRole("textbox", { name: "List name for this project" }).fill("Ops");
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await expect(page.getByText("Ops: 2 tasks added")).toBeVisible();

  await page.getByRole("link", { name: "Back to My Work" }).last().click();
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  await expect(page.getByText("Renew the certificates")).toBeVisible();
  await expect(page.getByText("Book the venue")).toBeVisible();
});
