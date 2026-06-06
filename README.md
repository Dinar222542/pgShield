# pgShield

PostgreSQL backup management server with scheduler, agent-based execution, Redis storage, and web dashboard.

## Features

- **Scheduler** — cron-based backup schedules with retention policy
- **Agent** — remote deployment for running pg_dump/pg_restore on database servers
- **Storage** — Local and NFS backends with connection testing
- **Monitoring** — real-time server, Redis, and system metrics with charts
- **Restore** — one-click restore from backup history
- **Auth** — JWT-based authentication with user management
- **Audit** — full action log for all operations

## Quick Start

```bash
# Build server
podman build -t localhost/pgshield-server:latest .

# Create network
podman network create pgshield-net

# Start Redis
podman run -d --name redis --network pgshield-net redis:7-alpine

# Start pgShield
podman run -d --privileged --name pgshield --network pgshield-net \
  -p 8080:8080 localhost/pgshield-server:latest
```

Dashboard: `http://localhost:8080` (default login: admin/admin)

## Components

| Component | Description |
|-----------|-------------|
| `pgshield-server` | Main server with web UI, scheduler, API |
| `pgshield-agent` | Remote agent for pg_dump/pg_restore execution |

## Configuration

Config file: `config/default.yaml`

- `auth.enabled` — toggle JWT authentication
- `database.redis_url` — Redis connection string
- `storage.backup_dir` — backup file storage path
- `metrics.ttl_days` — metrics retention period

## License

MIT
