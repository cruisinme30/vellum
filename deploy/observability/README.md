<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Observability

Vellum explains itself without any of this: Admin → Requests shows the last
10,000 requests as waterfalls, Admin → Routes shows rate, errors and
latency per route against its budget, and `vellum profile` draws a flame
graph. This folder is for running Vellum next to a metrics and tracing
stack. Decision record: ADR 0021.

| File | What it does |
| --- | --- |
| `otel-collector.yaml` | Receives OTLP from Vellum, exposes metrics to Prometheus and forwards traces and logs. |
| `prometheus/scrape.yaml` | Scrapes the collector and loads the rules. |
| `prometheus/rules.yaml` | RED recording rules, SLO burn-rate alerts, route budget and backup alerts. |
| `grafana/dashboards/` | RED and saturation per route, SLOs and error budgets, browser experience. |
| `grafana/provisioning/` | Loads the dashboards and the Prometheus data source. |

## Connect Vellum

Point the server at the collector:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4317 vellum serve
```

Set `TRACES_ENDPOINT` (for example Grafana Tempo on port 4317) and
`LOGS_ENDPOINT` (for example Loki's OTLP endpoint) for the collector.

## Alerts

| Alert | Fires when | Severity |
| --- | --- | --- |
| `Vellum*BudgetBurnFast` | An SLO burns its 30-day budget 14.4 times too fast over 1 h and 5 min | page |
| `Vellum*BudgetBurnSlow` | An SLO burns its budget 6 times too fast over 6 h and 30 min | ticket |
| `VellumHighErrorRate` | More than 1 % of requests fail for 5 min | page |
| `VellumRouteOverBudget` | A route's p95 is over its default budget for 10 min | ticket |
| `VellumBackupFailed`, `VellumBackupMissing` | A backup failed, or none succeeded in 26 h | ticket |
| `VellumRestoreDrillFailed`, `VellumRestoreDrillOverdue` | The monthly restore drill failed or did not run | page, ticket |

The SLOs match those the server evaluates itself: 99.9 % availability,
95 % of reads within 150 ms and 99 % of writes within 300 ms.

## Route budgets in CI

After a load test, fail the job when any route is over budget:

```sh
vellum budgets --server "$VELLUM_URL" --token "$VELLUM_ADMIN_TOKEN" --minutes 15
```

Pass `--budgets budgets.json` to check against budgets other than the
server's settings.
