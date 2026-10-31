---
id: 01M63CFE69CSEMP7N4AV85N8CD
title: Mermaid diagrams
vellum: 1
---

# Mermaid diagrams

Write Mermaid in a `mermaid` fence. Vellum supports every Mermaid
diagram type, renders it on the server, themes it to the page and wraps
long labels. Each type below shows the drawing, then its source. The
[Mermaid documentation](https://mermaid.js.org/intro/) covers every
option.

::: tip
Leave colours out of your diagrams. Vellum's theme colours them to match
the page in light and dark mode; colours you set in `classDef` or
`style` override the theme.
:::

## Flowchart

Boxes and arrows. Direction is `TB`, `LR`, `BT` or `RL`; shapes are `[box]`,
`(rounded)`, `{decision}`, `[(database)]` and more. Subgraphs group nodes, and
`classDef` styles them.

```mermaid
flowchart TB
  subgraph Edge
    GW[Gateway + CDN]
  end
  subgraph Stateless
    API[API] --> REN[Render]
    API --> EXP[Export]
  end
  subgraph Stateful
    COL[(Collab rooms)]
    IDX[(Index + search)]
  end
  GW --> API
  GW -. websocket .-> COL
  COL --> IDX
  REN --> BLOB[(Blob store)]
  API --> BLOB
  classDef store fill:#DDE1FF,stroke:#4F5B92;
  class COL,IDX,BLOB store;
```

`````markdown {corpus="mermaid/flowchart"}
```mermaid
flowchart TB
  subgraph Edge
    GW[Gateway + CDN]
  end
  subgraph Stateless
    API[API] --> REN[Render]
    API --> EXP[Export]
  end
  subgraph Stateful
    COL[(Collab rooms)]
    IDX[(Index + search)]
  end
  GW --> API
  GW -. websocket .-> COL
  COL --> IDX
  REN --> BLOB[(Blob store)]
  API --> BLOB
  classDef store fill:#DDE1FF,stroke:#4F5B92;
  class COL,IDX,BLOB store;
```
`````

## Sequence diagram

Messages between participants over time, with `alt`, `loop` and notes.

```mermaid
sequenceDiagram
  autonumber
  participant B as Browser
  participant G as Gateway
  participant A as API
  B->>G: GET /d/roadmap
  alt cached
    G-->>B: 200 page (ETag)
  else cache miss
    G->>A: render page
    A-->>G: html
    G-->>B: 200 page
  end
  loop every 30 s while idle
    B->>G: heartbeat
  end
  Note over G,A: Every hop carries the traceparent header
```

`````markdown {corpus="mermaid/sequence"}
```mermaid
sequenceDiagram
  autonumber
  participant B as Browser
  participant G as Gateway
  participant A as API
  B->>G: GET /d/roadmap
  alt cached
    G-->>B: 200 page (ETag)
  else cache miss
    G->>A: render page
    A-->>G: html
    G-->>B: 200 page
  end
  loop every 30 s while idle
    B->>G: heartbeat
  end
  Note over G,A: Every hop carries the traceparent header
```
`````

## State diagram

The states something moves through and the events that move it.

```mermaid
stateDiagram-v2
  [*] --> Draft
  Draft --> InReview: request review
  InReview --> Draft: changes requested
  InReview --> Verified: approve
  Verified --> Outdated: verify_every elapses
  Outdated --> Verified: re-verify
  Verified --> [*]
```

`````markdown {corpus="mermaid/state"}
```mermaid
stateDiagram-v2
  [*] --> Draft
  Draft --> InReview: request review
  InReview --> Draft: changes requested
  InReview --> Verified: approve
  Verified --> Outdated: verify_every elapses
  Outdated --> Verified: re-verify
  Verified --> [*]
```
`````

## Class diagram

Types, their fields and methods, and how they relate.

```mermaid
classDiagram
  class Page {
    +PageId id
    +String title
    +Status status
    +render() Html
  }
  class Book {
    +BookId id
    +Numbering numbering
    +nodes() List~Page~
  }
  class Collection {
    +Schema schema
    +rows() List~Page~
  }
  Book "1" o-- "*" Page : contains
  Collection "1" *-- "*" Page : rows
```

`````markdown {corpus="mermaid/class"}
```mermaid
classDiagram
  class Page {
    +PageId id
    +String title
    +Status status
    +render() Html
  }
  class Book {
    +BookId id
    +Numbering numbering
    +nodes() List~Page~
  }
  class Collection {
    +Schema schema
    +rows() List~Page~
  }
  Book "1" o-- "*" Page : contains
  Collection "1" *-- "*" Page : rows
```
`````

## Entity relationship diagram

Tables and their relationships, with crow's-foot cardinality.

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

`````markdown {corpus="mermaid/er"}
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
`````

## Gantt chart

Tasks on a calendar with dependencies and milestones. For a simpler roadmap, see
the [roadmap fence](../writing/plans.md#roadmap).

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

`````markdown {corpus="mermaid/gantt"}
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
`````

## Mind map

Ideas branching from a centre, by indentation.

```mermaid
mindmap
  root((Vellum 1.0))
    Write
      Pages
      Books
      Diagrams
    Organise
      Drive
      Collections
    Plan
      Issues
      Cycles
      My Work
    Connect
      MCP
      API
```

`````markdown {corpus="mermaid/mindmap"}
```mermaid
mindmap
  root((Vellum 1.0))
    Write
      Pages
      Books
      Diagrams
    Organise
      Drive
      Collections
    Plan
      Issues
      Cycles
      My Work
    Connect
      MCP
      API
```
`````

## Timeline

Events grouped by period.

```mermaid
timeline
  title Northstar gateway
  2025 : Prototype : First customer
  2026 : Policy engine : SOC 2
  2027 : Policy simulator : EU region
```

`````markdown {corpus="mermaid/timeline"}
```mermaid
timeline
  title Northstar gateway
  2025 : Prototype : First customer
  2026 : Policy engine : SOC 2
  2027 : Policy simulator : EU region
```
`````

## Quadrant chart

Items placed on two axes, split into four quadrants.

```mermaid
quadrantChart
  title Backlog by value and effort
  x-axis Low effort --> High effort
  y-axis Low value --> High value
  quadrant-1 Plan
  quadrant-2 Do now
  quadrant-3 Drop
  quadrant-4 Maybe
  Replay engine: [0.7, 0.9]
  Dark mode: [0.2, 0.6]
  CSV export: [0.15, 0.3]
  Rewrite proxy: [0.9, 0.35]
```

`````markdown {corpus="mermaid/quadrant"}
```mermaid
quadrantChart
  title Backlog by value and effort
  x-axis Low effort --> High effort
  y-axis Low value --> High value
  quadrant-1 Plan
  quadrant-2 Do now
  quadrant-3 Drop
  quadrant-4 Maybe
  Replay engine: [0.7, 0.9]
  Dark mode: [0.2, 0.6]
  CSV export: [0.15, 0.3]
  Rewrite proxy: [0.9, 0.35]
```
`````

## Sankey diagram

Flows between stages, as comma-separated `source,target,value` lines.

```mermaid
sankey-beta
Requests,Allowed,8200
Requests,Blocked,1800
Blocked,Injection,1100
Blocked,Rate limit,500
Blocked,Policy,200
```

`````markdown {corpus="mermaid/sankey"}
```mermaid
sankey-beta
Requests,Allowed,8200
Requests,Blocked,1800
Blocked,Injection,1100
Blocked,Rate limit,500
Blocked,Policy,200
```
`````

## XY chart

Bars and lines on shared axes.

```mermaid
xychart-beta
  title "Requests and errors"
  x-axis [Mon, Tue, Wed, Thu, Fri]
  y-axis "Requests (thousands)" 0 --> 120
  bar [80, 95, 102, 88, 110]
  line [4, 6, 5, 3, 2]
```

`````markdown {corpus="mermaid/xychart"}
```mermaid
xychart-beta
  title "Requests and errors"
  x-axis [Mon, Tue, Wed, Thu, Fri]
  y-axis "Requests (thousands)" 0 --> 120
  bar [80, 95, 102, 88, 110]
  line [4, 6, 5, 3, 2]
```
`````

## Git graph

Branches, commits and merges.

```mermaid
gitGraph
  commit id: "skeleton"
  branch develop
  checkout develop
  commit id: "kernel"
  branch feat/format
  checkout feat/format
  commit id: "parser"
  commit id: "corpus"
  checkout develop
  merge feat/format
  checkout main
  merge develop tag: "v1.0.0"
```

`````markdown {corpus="mermaid/gitgraph"}
```mermaid
gitGraph
  commit id: "skeleton"
  branch develop
  checkout develop
  commit id: "kernel"
  branch feat/format
  checkout feat/format
  commit id: "parser"
  commit id: "corpus"
  checkout develop
  merge feat/format
  checkout main
  merge develop tag: "v1.0.0"
```
`````

## Pie chart

Shares of a whole. The [chart fence](../writing/charts.md#pie) draws the same
with more options.

```mermaid
pie showData
  title Time spent this cycle
  "Features" : 55
  "Bugs" : 30
  "Chores" : 15
```

`````markdown {corpus="mermaid/pie"}
```mermaid
pie showData
  title Time spent this cycle
  "Features" : 55
  "Bugs" : 30
  "Chores" : 15
```
`````

## User journey

Steps a person takes and how each one feels, scored 1 to 5.

```mermaid
journey
  title Writing a status update
  section Draft
    Open last week's page: 4: Priya
    Copy the template: 3: Priya
  section Review
    Ask for comments: 4: Priya, Dev
    Resolve comments: 2: Priya
  section Share
    Post the link: 5: Priya
```

`````markdown {corpus="mermaid/journey"}
```mermaid
journey
  title Writing a status update
  section Draft
    Open last week's page: 4: Priya
    Copy the template: 3: Priya
  section Review
    Ask for comments: 4: Priya, Dev
    Resolve comments: 2: Priya
  section Share
    Post the link: 5: Priya
```
`````

## Requirement diagram

Requirements, the things that satisfy them and how they are verified.

```mermaid
requirementDiagram
  requirement replay {
    id: R1
    text: Replay 7 days of traffic in under 10 minutes
    risk: medium
    verifymethod: test
  }
  element replay_engine {
    type: service
  }
  element load_test {
    type: "load test"
  }
  replay_engine - satisfies -> replay
  load_test - verifies -> replay
```

`````markdown {corpus="mermaid/requirement"}
```mermaid
requirementDiagram
  requirement replay {
    id: R1
    text: Replay 7 days of traffic in under 10 minutes
    risk: medium
    verifymethod: test
  }
  element replay_engine {
    type: service
  }
  element load_test {
    type: "load test"
  }
  replay_engine - satisfies -> replay
  load_test - verifies -> replay
```
`````

## C4 diagram

System context, container and component views in the C4 model.

```mermaid
C4Context
  title Northstar gateway: system context
  Person(dev, "API developer", "Calls Northstar APIs")
  System(gw, "Northstar gateway", "Authenticates, rate limits and audits requests")
  System_Ext(idp, "Identity provider", "Issues tokens")
  System_Ext(backend, "Customer backend", "Serves the API")
  Rel(dev, gw, "Calls", "HTTPS")
  Rel(gw, idp, "Validates tokens")
  Rel(gw, backend, "Forwards allowed requests")
```

`````markdown {corpus="mermaid/c4"}
```mermaid
C4Context
  title Northstar gateway: system context
  Person(dev, "API developer", "Calls Northstar APIs")
  System(gw, "Northstar gateway", "Authenticates, rate limits and audits requests")
  System_Ext(idp, "Identity provider", "Issues tokens")
  System_Ext(backend, "Customer backend", "Serves the API")
  Rel(dev, gw, "Calls", "HTTPS")
  Rel(gw, idp, "Validates tokens")
  Rel(gw, backend, "Forwards allowed requests")
```
`````

## Block diagram

Blocks laid out on a grid that you control.

```mermaid
block-beta
  columns 3
  client["Client"] space gateway["Gateway"]
  space space space
  db[("Audit log")] space backend["Backend"]
  client --> gateway
  gateway --> backend
  gateway --> db
```

`````markdown {corpus="mermaid/block"}
```mermaid
block-beta
  columns 3
  client["Client"] space gateway["Gateway"]
  space space space
  db[("Audit log")] space backend["Backend"]
  client --> gateway
  gateway --> backend
  gateway --> db
```
`````

## Packet diagram

The fields of a network packet or binary format, by bit range.

```mermaid
packet-beta
  title Share token record
  0-15: "Version"
  16-31: "Flags"
  32-95: "Expiry (Unix ms)"
  96-127: "Space ID prefix"
```

`````markdown {corpus="mermaid/packet"}
```mermaid
packet-beta
  title Share token record
  0-15: "Version"
  16-31: "Flags"
  32-95: "Expiry (Unix ms)"
  96-127: "Space ID prefix"
```
`````

## Architecture diagram

Cloud services and how they connect, with groups and icons.

```mermaid
architecture-beta
  group cloud(cloud)[Northstar cloud]
  service gw(internet)[Gateway] in cloud
  service api(server)[API] in cloud
  service db(database)[Postgres] in cloud
  service blob(disk)[Blob store] in cloud
  gw:R --> L:api
  api:R --> L:db
  api:B --> T:blob
```

`````markdown {corpus="mermaid/architecture"}
```mermaid
architecture-beta
  group cloud(cloud)[Northstar cloud]
  service gw(internet)[Gateway] in cloud
  service api(server)[API] in cloud
  service db(database)[Postgres] in cloud
  service blob(disk)[Blob store] in cloud
  gw:R --> L:api
  api:R --> L:db
  api:B --> T:blob
```
`````

## Kanban

Columns of cards, for a snapshot of work.

```mermaid
kanban
  todo[To do]
    t1[Write the migration guide]
  doing[Doing]
    t2[Replay engine]@{ assigned: "Dev", priority: "High" }
  done[Done]
    t3[Diff report]
```

`````markdown {corpus="mermaid/kanban"}
```mermaid
kanban
  todo[To do]
    t1[Write the migration guide]
  doing[Doing]
    t2[Replay engine]@{ assigned: "Dev", priority: "High" }
  done[Done]
    t3[Diff report]
```
`````

## Radar chart

Several measures on spokes from a centre, to compare profiles.

```mermaid
radar-beta
  title Gateway options
  axis perf["Performance"], cost["Cost"], ops["Operability"], sec["Security"], fit["Fit"]
  curve a["Build"]{4, 2, 3, 5, 5}
  curve b["Buy"]{3, 4, 4, 4, 3}
  max 5
```

`````markdown {corpus="mermaid/radar"}
```mermaid
radar-beta
  title Gateway options
  axis perf["Performance"], cost["Cost"], ops["Operability"], sec["Security"], fit["Fit"]
  curve a["Build"]{4, 2, 3, 5, 5}
  curve b["Buy"]{3, 4, 4, 4, 3}
  max 5
```
`````

## Treemap

Nested rectangles sized by value.

```mermaid
treemap-beta
"Storage"
  "Pages": 420
  "Assets": 310
  "Index": 90
"Compute"
  "Render": 160
  "Search": 120
```

`````markdown {corpus="mermaid/treemap"}
```mermaid
treemap-beta
"Storage"
  "Pages": 420
  "Assets": 310
  "Index": 90
"Compute"
  "Render": 160
  "Search": 120
```
`````
