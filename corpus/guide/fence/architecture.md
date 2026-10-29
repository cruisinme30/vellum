```architecture
Clients: Web app, Mobile app, MCP client
Edge: Gateway
Services: API, Render, Worker
Data: Postgres, Object store
Web app -> Gateway
Mobile app -> Gateway
MCP client -> Gateway
Gateway -> API
API -> Render
API -> Postgres
Worker -> Object store
Render -> Object store
```
