// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { previewCount } from "./import-preview";

describe("previewCount", () => {
  it("counts Google Tasks items, not their lists", () => {
    const takeout = {
      kind: "tasks#taskLists",
      items: [
        {
          kind: "tasks#taskList",
          title: "Errands",
          status: "x",
          items: [
            { kind: "tasks#task", title: "Milk", status: "needsAction" },
            { kind: "tasks#task", title: "Bread", status: "completed" },
          ],
        },
      ],
    };
    expect(previewCount("google_tasks", JSON.stringify(takeout))).toBe(2);
  });

  it("counts Things to-dos and checklist items", () => {
    const things = [
      { type: "to-do", attributes: { title: "A" } },
      {
        type: "project",
        attributes: {
          title: "P",
          items: [
            {
              type: "to-do",
              attributes: {
                title: "B",
                "checklist-items": [{ type: "checklist-item" }],
              },
            },
          ],
        },
      },
    ];
    expect(previewCount("things", JSON.stringify(things))).toBe(3);
  });

  it("counts Todoist task rows and refuses other files", () => {
    const csv =
      '"TYPE",CONTENT\r\ntask,One\r\nsection,Later\r\n"task",Two\r\nnote,x';
    expect(previewCount("todoist", csv)).toBe(2);
    expect(previewCount("todoist", "a,b\n1,2")).toBeUndefined();
    expect(previewCount("google_tasks", "not json")).toBeUndefined();
    expect(previewCount("things", "{}")).toBe(0);
  });
});
