# Contributing to notifyd

Thanks for your interest in contributing! notifyd is a self-hosted notification service built with Rust, and we welcome contributions of all kinds — bug fixes, new connectors, documentation improvements, or feature ideas.

## Code of Conduct

Be respectful, constructive, and focused on the technical merits. We're here to build good software together.

## Getting Started

### Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| **Rust** | 1.75+ | [rustup.rs](https://rustup.rs) |
| **PostgreSQL** | 16+ | `brew install postgresql` / `apt install postgresql` |
| **Docker** (optional) | 24+ | [docker.com](https://docker.com) |

### Setup

```bash
# 1. Fork and clone
git clone https://github.com/YOUR_USERNAME/notifyd.git
cd notifyd

# 2. Configure
cp notifyd.toml.example notifyd.toml
# Edit notifyd.toml — set database.url to your local Postgres

# 3. Create the database
createdb notifyd

# 4. Run (migrations run automatically on startup)
cargo run

# 5. Test
cargo test
```

### Using Docker for Postgres

If you don't have Postgres installed locally:

```bash
docker compose up -d postgres
# Then run notifyd natively:
export DATABASE_URL=postgres://notifyd:notifyd@localhost:5432/notifyd
cargo run
```

## How to Contribute

### Bug Reports

Open an issue with:
- What you expected vs. what happened
- Steps to reproduce
- notifyd version (`/v1/health` returns it)
- Relevant logs (with PII redacted)

### Feature Requests

Open an issue describing:
- The problem you're solving
- Why the current behavior is insufficient
- Your proposed API/behavior (even rough)

For large features, **discuss before implementing**. This saves everyone time.

### Pull Requests

1. **Fork** the repo and create a branch from `main`
2. **Write code** following the project conventions (see below)
3. **Add tests** for new functionality
4. **Update docs** — if you add an endpoint, update `docs/API.md` and `docs/llms.txt`
5. **Open a PR** with a clear description of what and why

### Commit Messages

We use conventional commits:

```
feat: add Mailgun email connector
fix: handle empty subscriber_id in batch send
docs: add webhook setup guide
refactor: extract job retry logic to separate module
```

## Project Conventions

### Code Style

- **File size**: keep files under 400 lines. Split when approaching.
- **Function size**: under 50 lines. Extract helpers.
- **Naming**: explicit, boring names. `extract_project()` not `get_p()`.
- **Error handling**: use `anyhow::Result` internally, return proper HTTP status codes at the API boundary.
- **SQL**: raw `sqlx::query` with bind params. No ORM.

### Architecture Decisions

| Decision | Rationale |
|----------|-----------|
| Postgres-only queue | `SELECT FOR UPDATE SKIP LOCKED` is fast enough for most workloads. No Redis complexity. |
| Single binary | Easier to deploy, monitor, and debug than a microservice mesh. |
| TOML config | Simple, typed, no YAML surprises. |
| SSE over WebSocket | Unidirectional (server→client) is all we need. Simpler, works through proxies. |
| Axum | Best async Rust web framework. Tower middleware, extractors, great ergonomics. |

### Adding a New Connector

1. Create `src/connectors/your_connector.rs`
2. Implement the `Connector` trait:

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    async fn send(&self, request: SendRequest) -> Result<()>;
}
```

3. Add it to `src/connectors/mod.rs`
4. Wire it in `src/worker.rs` (`process_job` match)
5. Add config section in `src/config.rs`
6. Update `notifyd.toml.example`
7. Update `docs/API.md` and `docs/llms.txt`
8. Open a PR 🎉

### Adding a New API Endpoint

1. Create the handler in `src/api/your_module.rs`
2. Register the route in `src/api/mod.rs`
3. Add the endpoint to `docs/API.md` (with curl + TypeScript examples)
4. Add the endpoint to `docs/llms.txt`
5. Update the README API table if it's a key endpoint

## Testing

```bash
# Run all tests
cargo test

# Run with logging
RUST_LOG=notifyd=debug cargo test

# Test a specific module
cargo test api::send
```

### Writing Tests

- Unit tests go in the same file (Rust convention)
- Integration tests that need a DB go in `tests/`
- Mock external connectors (Resend, Twilio) — don't call real APIs in tests

## Documentation

We maintain three layers of documentation:

| File | Audience | What to update |
|------|----------|----------------|
| `README.md` | First-time visitors | Feature list, quick start, high-level API |
| `docs/API.md` | Developers integrating | Endpoint details, request/response, examples |
| `docs/llms.txt` | AI agents | Full reference in plain text (LLM-friendly) |
| `docs/SETUP.md` | Contributors | Local dev setup, config options |
| `docs/ARCHITECTURE.md` | Contributors | Design decisions, internals |

When changing the API, update **all three**: API.md, llms.txt, and README if relevant.

## Release Process

Releases are cut from `main`. Version follows semver in `Cargo.toml`.

1. Update version in `Cargo.toml`
2. Update CHANGELOG.md
3. Tag: `git tag v0.2.0 && git push --tags`
4. Docker image builds automatically

## Questions?

Open an issue or reach out. We're friendly.

---

Built with 🦀 in Grenoble, France 🏔️
