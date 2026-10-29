```mermaid
flowchart LR
  A[Request] --> B{Allowed?}
  B -->|yes| C[Serve]
  B -->|no| D[Block]
```

```d2
gateway -> engine: evaluate
```

```plantuml
@startuml
Alice -> Bob: hello
@enduml
```
