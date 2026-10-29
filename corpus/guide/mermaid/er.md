```mermaid
erDiagram
  TEAM ||--o{ ISSUE : owns
  ISSUE }o--o| CYCLE : "planned in"
  ISSUE }o--o{ LABEL : tagged
  USER ||--o{ ISSUE : assigned
  ISSUE {
    string key PK
    string title
    int priority
    date due
  }
```
