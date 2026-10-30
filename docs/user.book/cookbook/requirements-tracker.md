---
id: 01M63CFE6FECTX40YFDXFNMDJN
title: A requirements tracker
vellum: 1
---

# A requirements tracker

**You will build** a collection of requirements with priorities,
owners, targets and a computed "points left", a board by status, and a
release collection that rolls the points up.

## Steps

1. Select **New → Collection** and call it "Requirements tracker".
2. Add properties: Priority (select: P0, P1, P2), Status (select: Open, In
   review, Done, Blocked), Owner (person), Target (date), Estimate (number) and
   Progress (number, shown as percent).
3. Add a formula property "Remaining" with `round(Estimate * (1 - Progress), 1)`.
4. Create a second collection, "Releases", and add a relation from requirements
   to releases.
5. On Releases, add a rollup "Total points" that sums Estimate through the
   relation.
6. Add a **Board** view grouped by Status.
7. Embed the board in the project page with `/collection`.

## The finished embed

```collection
id: 01JB4Q8ZK2W6T9R3M5N7P1X0YC
view: By status
filter: Status != Done
```

See [Collections](../data/README.md) for the full schema and
[Formulas](../data/formulas.md) for 30 more formulas.
