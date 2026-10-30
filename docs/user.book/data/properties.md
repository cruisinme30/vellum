---
id: 01M63CFE6AVJJ8BPGV8PSJH467
title: Properties and relations
vellum: 1
---

# Properties and relations

## Property types

| Type | Holds | Example |
| --- | --- | --- |
| `title` | The row's name; every collection has exactly one | R1 · Replay 7 days |
| `text` | Free text | Owned by the gateway team |
| `number` | A number, shown as plain, percent or currency | 8 |
| `select` | One option from a list | P0 |
| `multi` | Several options from a list | gateway, audit |
| `person` | One or more people | Dev Rao |
| `date` | A date, with an optional time and end | 2026-11-24 |
| `checkbox` | Yes or no | ✓ |
| `url` | A web address | `https://northstar.dev` |
| `relation` | Links to rows in another collection | 1.0 Policy simulator |
| `rollup` | A summary of a property across related rows | 16 points |
| `formula` | A value computed from other properties | 1.5 |
| `created` | Who created the row and when | Priya Shah, 2026-10-02 |

Select **Properties** on a collection to rename a property, change its
type, change the options of a select or a formula, delete a property, or add
a new one at the end. Renaming a property updates the formulas that use
it. Changing the type keeps every value in the files: values that do not
fit the new type are listed under **Problems** until you fix them or
change the type back. Titles and relations keep their type. Deleting a
property also removes the filters, sorts and grouping that used it;
Vellum refuses while a formula or rollup still reads it.

## Relations {#relations}

A relation links rows in one collection to rows in another. Relations
are always two-way: adding a requirement to a release adds the release
to the requirement. To add one, open **Properties**, pick **Relation**,
choose the collection it links to and name the matching property there;
you need to be able to edit both collections. Deleting a relation
removes it from both. In the schema, `to:` names the other collection by
ID.

```yaml
- {key: rel, name: Release, type: relation, to: 01JB5C2V8N4R6T0W3Y5Z7Q9K1M}
```

## Rollups {#rollups}

A rollup summarises a property of the related rows. In **Properties**,
pick **Rollup**, then the relation, the property of the linked rows and
how to combine them. On the releases collection, this rollup adds up the
estimates of each release's requirements:

```yaml
- {key: pts, name: Total points, type: rollup, via: rq, prop: est, agg: sum}
```

| Aggregation | Result |
| --- | --- |
| count | Number of related rows |
| sum, avg, min, max | Arithmetic over a number property |
| checked | Number of related rows with a ticked checkbox |
| pct | Share of related rows with a ticked checkbox, as a percent |
| unique | Number of distinct values |
| show | The values themselves, as a list |
