```plantuml
@startuml
actor Developer
participant Gateway
database "Audit log" as Audit
Developer -> Gateway: POST /orders
Gateway -> Audit: write decision
Gateway --> Developer: 201 Created
@enduml
```
