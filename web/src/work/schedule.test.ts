// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  addDays,
  blockEnd,
  formatBlock,
  isMovable,
  moveTargets,
  parseBlock,
  sectionTarget,
} from "./schedule";

describe("schedule", () => {
  it("counts days across months and years", () => {
    expect(addDays("2026-10-31", 1)).toBe("2026-11-01");
    expect(addDays("2026-12-28", 7)).toBe("2027-01-04");
    expect(moveTargets("2026-10-29").map((target) => target.to)).toEqual([
      "inbox",
      "today",
      "evening",
      "2026-10-30",
      "2026-11-05",
      "anytime",
      "someday",
    ]);
  });

  it("moves to a section's bucket or day, and never to Done", () => {
    expect(sectionTarget({ bucket: "today", tasks: [] })).toBe("today");
    expect(
      sectionTarget({ bucket: "upcoming", day: "2026-11-02", tasks: [] }),
    ).toBe("2026-11-02");
    expect(
      sectionTarget({ bucket: "upcoming", week_of: "2026-11-09", tasks: [] }),
    ).toBe("2026-11-09");
    expect(sectionTarget({ bucket: "done", tasks: [] })).toBeUndefined();
    expect(isMovable({ kind: "checklist", id: "D1.B1" })).toBe(true);
    expect(isMovable({ kind: "issue", id: "issue.ENG-7" })).toBe(false);
  });

  it("reads and writes time blocks", () => {
    expect(parseBlock("2026-11-03T15:00/90")).toEqual({
      start: "2026-11-03T15:00",
      minutes: 90,
    });
    expect(parseBlock("2026-11-03T15:00/0")).toBeUndefined();
    expect(parseBlock("2026-11-03T15:00/1441")).toBeUndefined();
    expect(parseBlock("tomorrow")).toBeUndefined();
    expect(parseBlock()).toBeUndefined();
    expect(formatBlock("2026-11-03T15:00:00", 45)).toBe("2026-11-03T15:00/45");
    expect(formatBlock("", 45)).toBeUndefined();
    expect(blockEnd({ start: "2026-11-03T15:00", minutes: 90 })).toBe("16:30");
    expect(blockEnd({ start: "2026-11-03T23:30", minutes: 60 })).toBe("00:30");
  });
});
