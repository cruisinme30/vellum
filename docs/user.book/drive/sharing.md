---
id: 01M63CFE6EBPZZV9796TFPBFZQ
title: Sharing and permissions
vellum: 1
---

# Sharing and permissions

Nobody sees anything in Vellum unless someone gave them access. Access
is granted to people, groups or everyone in the organisation, on a
drive, folder or page.

## Roles {#roles}

| Role | Can |
| --- | --- |
| Can view | Read the page and its comments |
| Can comment | View, plus comment and suggest changes |
| Can edit | Comment, plus change the page |
| Full access | Edit, plus share, move and delete |

## Inheritance {#inheritance}

Access flows down: sharing a folder shares everything inside it. A page
can add access on top of what it inherits. A page can also be
**restricted**, so that only the people named on it can open it, even
if they have access to the folder.

A folder's access is also written to its `.vellum-folder.yaml`, so the
files on disk say who may read them:

```yaml
id: 01JB8E5G7J9L1N3Q6S8U0W2Y4A
icon: 📁
access:
  everyone: view
  group:product: edit
```

## Public links {#public-links}

**Share → Anyone with the link** creates a link that works without
signing in. Set an expiry date and turn the link off at any time. Public
pages are served from a separate address from the app, and readers of a
public link can never see comments or anything else in the workspace.

## Checking access {#check}

**Share → Who has access** lists everyone who can open the page and
why: directly, through a group or through a folder.
