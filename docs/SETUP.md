# Development Setup Guide

This guide covers setting up notifyd for local development and production deployment.

---

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| **Rust** | 1.75+ | Compiler + cargo |
| **PostgreSQL** | 16+ | Database |
| **Docker** (optional) | 24+ | Container builds |

### Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Install PostgreSQL

```bash
# macOS
brew install postgresql@16 && brew services start postgresql@16

# Ubuntu/Debian
sudo apt install postgresql-16

# Or just use Docker (see below)
```

---

## Local Development

### 1. Clone & Configure

```bash
git clone https://github.com/rmzlb/notifyd.git
cd notifyd
cp notifyd.toml.example notifyd.toml
```

Edit `notifyd.toml`:

```toml
[server]
port = 3400
jwt_secret = "dev-secret-change-in-prod"

[database]
url = "postgres://notifyd:notifyd@localhost:5432/notifyd"
max_connections = 5

[worker]
poll_interval_ms = 500
batch_size = 50
max_attempts = 3

[connectors.email]
provider = "resend"
api_key = "re_test_xxx"        # Get a free key at resend.com
from = "dev@yourdomain.com"

[projects.dev]
api_key = "sk_dev_test123"
channels = ["email", "in_app"]
```

### 2. Database Setup

**Option A: Local Postgres**

```bash
createdb notifyd
# Migrations run automatically on startup
```

**Option B: Docker Postgres**

```bash
docker compose up -d postgres
# Uses postgres://notifyd:notifyd@localhost:5432/notifyd
```

### 3. Run

```bash
# Development (with hot reload via cargo-watch)
cargo install cargo-watch
cargo watch -x run

# Or just
cargo run
```

notifyd starts on `http://localhost:3400`. Migrations run automatically.

### 4. Verify

```bash
curl http://localhost:3400/v1/health
# → {"status":"ok","db":"ok","version":"0.1.0"}
```

### 5. Test

```bash
cargo test

# With debug logging
RUST_LOG=notifyd=debug cargo test

# Specific module
cargo test api::send
```

---

## Docker Build

notifyd uses a multi-stage Docker build with `cargo-chef` for optimal layer caching:

```bash
# Build
docker build -t notifyd .

# Run
docker run -p 3400:3400 \
  -e DATABASE_URL=postgres://user:pass@host:5432/notifyd \
  -v ./notifyd.toml:/app/notifyd.toml:ro \
  notifyd
```

### docker-compose (full stack)

```bash
docker compose up -d
# Starts notifyd + PostgreSQL 16
# notifyd available at http://localhost:3400
```

---

## Environment Variables

notifyd reads config from `notifyd.toml` but these env vars override:

| Variable | Override | Description |
|----------|----------|-------------|
| `DATABASE_URL` | `database.url` | PostgreSQL connection string |
| `PORT` | `server.port` | Listen port (default: 3400) |
| `NOTIFYD_CONFIG` | — | Path to TOML config file |
| `RUST_LOG` | — | Log level (`notifyd=info`, `notifyd=debug`) |
| `JWT_SECRET` | `server.jwt_secret` | JWT signing secret |
| `RESEND_API_KEY` | `connectors.email.api_key` | Resend API key |
| `EMAIL_FROM` | `connectors.email.from` | Default sender email |

---

## Production Deployment

### Docker (recommended)

```bash
# 1. Build optimized image
docker build -t notifyd:latest .

# 2. Create config
cp notifyd.toml.example notifyd.toml
# Edit with real credentials

# 3. Run with external Postgres
docker run -d --name notifyd \
  --restart unless-stopped \
  -p 3400:3400 \
  -e DATABASE_URL=postgres://user:pass@db-host:5432/notifyd \
  -v /path/to/notifyd.toml:/app/notifyd.toml:ro \
  notifyd:latest
```

### Behind a Reverse Proxy (Traefik, nginx, Caddy)

notifyd serves HTTP on the configured port. Put it behind your reverse proxy for TLS.

**Caddy example:**

```
notifications.example.com {
    reverse_proxy localhost:3400
}
```

**nginx example:**

```nginx
server {
    listen 443 ssl;
    server_name notifications.example.com;

    location / {
        proxy_pass http://localhost:3400;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

        # SSE support
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 86400s;
    }
}
```

> ⚠️ **Important for SSE**: disable response buffering (`proxy_buffering off`) so realtime events stream correctly.

### Health Checks

```bash
# Liveness
curl http://localhost:3400/v1/health
# → {"status":"ok","db":"ok","version":"0.1.0"}

# Metrics (admin key required)
curl http://localhost:3400/v1/metrics \
  -H "X-Api-Key: admin_xxx"
```

---

## Database Migrations

Migrations are in `migrations/` and run automatically on startup:

| File | Description |
|------|-------------|
| `001_init.sql` | Core tables: projects, subscribers, jobs, inbox, templates |
| `002_preferences_workflows.sql` | Preferences + workflow engine |
| `003_projects_api.sql` | Admin API, key rotation, audit log |
| `004_webhooks.sql` | Webhook delivery |

No migration tool needed. notifyd handles it internally.

---

## Troubleshooting

### "Connection refused" on startup

→ Check PostgreSQL is running and `database.url` is correct in `notifyd.toml`.

### SSE stream closes immediately

→ If behind a reverse proxy, ensure `proxy_buffering off` is set. SSE needs unbuffered responses.

### "API key not found"

→ Keys are defined in `notifyd.toml` under `[projects.xxx]`. Make sure the project section exists.

### "Rate limited" (429)

→ Default is 100 req/min per project. Adjust in the project config or contact the admin.
