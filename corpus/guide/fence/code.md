```bash
helm rollback gateway 1 -n prod
kubectl rollout status deploy/gateway
```
