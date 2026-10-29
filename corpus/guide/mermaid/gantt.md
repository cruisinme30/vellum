```mermaid
gantt
  title Policy simulator
  dateFormat YYYY-MM-DD
  section Build
    Replay engine      :done,    re, 2026-10-01, 2026-10-28
    Diff report        :active,  dr, after re, 14d
    PDF report         :         pr, after dr, 10d
  section Launch
    Beta               :         b,  after pr, 21d
    GA                 :milestone, ga, after b, 0d
```
