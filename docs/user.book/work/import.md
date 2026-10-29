---
id: 01M63CFE6D1GYA247XRQD5N8XV
title: Importing from Linear and Jira
vellum: 1
---

# Importing from Linear and Jira

**Admin → Import** brings issues in from Linear, Jira and GitHub Issues.
Each import keeps:

- keys, as aliases, so old links and commit messages still resolve
- titles, descriptions (converted to Vellum Markdown), comments and attachments
- statuses, mapped to the seven categories; you confirm the mapping before the import runs
- assignees, matched by email
- labels, priorities, estimates, cycles or sprints, projects and relations

Run an import as a dry run first: Vellum reports what it would create
and any fields it cannot map, and writes nothing.
