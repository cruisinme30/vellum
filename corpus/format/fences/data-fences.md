```timeline
2026-10-01 | Beta opens | Invite-only, Northstar design partners
2026-11-24 | General availability
```

```roadmap
range: 2026-10-01 .. 2027-03-31
Replay engine | 2026-10-01 | 2026-12-15
GA | 2026-11-24 | milestone
```

```risk-matrix
Data loss | likelihood 1 | impact 5
Slow search | likelihood 3 | impact 2
```

```board
## Todo
- Write runbook [P1]
## Doing
- Replay engine
## Done
- Diff report
```

```table sortable bar=Revenue sum=Revenue
Region,Revenue
EMEA,120
APAC,90
```

```architecture
Edge: Gateway
Core: Policy engine, Audit log
Gateway -> Policy engine: checks
Policy engine -> Audit log
```
