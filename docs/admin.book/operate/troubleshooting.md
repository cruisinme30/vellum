---
id: 01M63CFE622VGV774X6YCXDYCP
title: Troubleshooting
vellum: 1
---

# Troubleshooting

Start with `vellum doctor`. It checks the configuration, storage,
index, keys, Kroki and the network, and prints what is wrong and how to
fix it.

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `/readyz` returns 503 "index not open" | The index is being migrated or rebuilt | Wait; the log shows progress |
| Pages missing after editing files by hand | The watcher missed changes on a network share | `vellum reindex --drive <name>` |
| "Kroki unavailable" on diagrams | Kroki is not running or not reachable | Check `VELLUM_KROKI_URL` and the Kroki container |
| Conflict copies appear | Two people edited the same lines in and out of Vellum | Expected; merge the copy into the page |
| Users cannot sign in with SSO | Clock skew or a changed client secret | Check time sync and **Admin → Sign-in** |
| Slow pages for one space | A very large page or collection | Open **Admin → Requests**, sort by slowest, follow the trace |

## Rebuilding the index {#reindex}

```bash
vellum reindex            # every drive
vellum reindex --drive product
```

The index is derived from the files, so rebuilding it is always safe.
Per-person state such as stars and recent pages is kept.

## Getting help {#help}

Include the request ID from the error dialog and the output of
`vellum doctor --redact` when you report a problem.
