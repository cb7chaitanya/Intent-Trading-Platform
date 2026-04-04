# Log Aggregation with Loki

## Architecture

```
Rust services (JSON stdout) → Docker → Promtail → Loki → Grafana
```

All Rust services emit structured JSON logs to stdout when running in docker/production. Promtail discovers containers via Docker socket and ships logs to Loki.

## Setup

```bash
docker compose up -d loki promtail grafana
```

Grafana: `http://localhost:3002` (admin/admin)
Loki datasource is auto-provisioned.

## Example LogQL Queries

### All logs from platform service
```logql
{service="intent-trading"}
```

### Errors only
```logql
{service="intent-trading"} |= "ERROR" | json | level="ERROR"
```

### Settlement events
```logql
{service="intent-trading"} | json | message=~"settle.*"
```

### Specific intent
```logql
{service="intent-trading"} | json | intent_id="<uuid>"
```

### Slow API requests (>500ms)
```logql
{service="intent-trading"} | json | duration_ms > 500
```

### Auction lifecycle
```logql
{service="intent-trading"} | json | message=~"auction.*"
```

### All errors across all services
```logql
{project="intent-trading"} | json | level="ERROR"
```

### Rate limit violations
```logql
{service="api-gateway"} |= "rate_limit"
```

### TWAP progress
```logql
{service="intent-trading"} | json | message=~"twap.*"
```

### Log rate by service
```logql
sum by (service) (rate({project="intent-trading"}[5m]))
```

## JSON Log Format

Rust tracing with `.json()` produces:
```json
{
  "timestamp": "2026-04-05T12:00:00.000Z",
  "level": "INFO",
  "target": "intent_trading::engine::auction_engine",
  "fields": {
    "message": "auction_matched",
    "intent_id": "abc-123",
    "solver_id": "solver-1",
    "amount_out": 950
  },
  "span": {
    "name": "..."
  }
}
```

Promtail extracts `intent_id`, `trade_id`, `solver_id`, `user_id`, `duration_ms`, `error` as labels for fast filtering.

## Retention

Loki retains logs for 7 days (configurable in `loki-config.yml`).
