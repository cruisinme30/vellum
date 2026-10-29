```timeline
2026-10-01: Beta opens
2026-11-24: General availability
```

```roadmap
range: 2026-10 .. 2027-03
Replay engine: 2026-10 .. 2026-12
milestone GA: 2026-11-24
```

```risk-matrix
Data loss: likelihood 1, impact 5
Slow search: likelihood 3, impact 2
```

```board
Todo: Write runbook
Doing: Replay engine
Done: Diff report
```

```table sortable bar=Revenue sum=Revenue
Region,Revenue
EMEA,120
APAC,90
```

```architecture
lane Edge: Gateway
lane Core: Policy engine, Audit log
Gateway -> Policy engine
Policy engine -> Audit log
```
