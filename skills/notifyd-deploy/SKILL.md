---
name: notifyd-deploy
description: Deploy and configure a notifyd instance (one Rust binary + Postgres) with Docker Compose or Dokploy — one instance per company, providers by environment variables (Resend, Cloudflare Email Service, SMTP, Telnyx, Twilio), fallback provider, pacing, public URL, admin and read-only keys, MCP access. Use when standing up notifyd, adding a provider, tuning rate limits, or diagnosing a deployment.
license: MIT
metadata:
  author: rmzlb
  version: "1.0"
---

# Deploying notifyd

Read `docs/DEPLOYMENTS.md` (topology, inventory, runbook) and
`docs/CONNECTORS.md` (every variable). Principle: **one instance per
company**, all from the same repository, everything specific in environment
variables, never in the repo.

## Minimal production environment

| Variable | Purpose |
|---|---|
| `DATABASE_URL` (or `POSTGRES_PASSWORD` with the bundled Postgres) | Postgres 16 |
| `JWT_SECRET` | signs subscriber tokens and unsubscribe links (32+ random bytes) |
| `ADMIN_API_KEY` | operator surface + MCP, full rights |
| `READONLY_API_KEY` (optional) | digest, listings, metrics, read-only MCP tools |
| `EMAIL_PROVIDER` + credentials | `resend` (`RESEND_API_KEY`), `cloudflare` (`CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_EMAIL_API_TOKEN`), `smtp` (`SMTP_HOST`…), `agentmail`, `log` (dev only) |
| `EMAIL_FROM`, `EMAIL_FROM_NAME` | default sender, domain verified at the provider |
| `EMAIL_FALLBACK_PROVIDER` + its credentials | second provider used on 429/5xx; same sender domain verified there too |
| `PUBLIC_URL` | public base URL of the instance: hosts the one-click unsubscribe links |
| `CORS_ORIGINS` | back-office origins allowed to open the in-app stream and call `/mcp` from a browser |
| `SMS_PROVIDER` + `SMS_FROM` + `TELNYX_API_KEY` or `TWILIO_*` | optional SMS; `TELNYX_WHATSAPP_API_KEY` + `WHATSAPP_FROM` for WhatsApp |
| `EMAIL_RATE_PER_SEC` (8), `WORKER_MAX_ATTEMPTS` (5) | pacing under the provider limit, attempts before `failed` |

Generate secrets with `openssl rand -hex 32`. Never write a value in a file
tracked by git.

## Steps

1. Create the compose service from `docker-compose.yml` of this repository
   (Dokploy: Compose, git source, branch `main`, autodeploy on; add a push
   webhook on the repository → `<dokploy>/api/deploy/compose/<token>` when
   the Dokploy instance has no GitHub App).
2. Set the variables above, deploy, check `GET /v1/health`: `status: ok`,
   `commit` equal to `main`.
3. Create the company project: `POST /v1/admin/projects` with `id`, `name`,
   `channels`, `from_email`, `from_name`. Store the returned key as the
   application's `NOTIFYD_API_KEY`. Delete the seeded `craie` project on any
   instance that is not CRAIE's (`DELETE /v1/admin/projects/craie`).
4. Point the application at `http://notifyd:3400` when it runs on the same
   Docker network; give browsers the public URL only for the in-app stream.
5. Run `GET /v1/admin/digest` (or the `digest` MCP tool): the findings tell
   you what is still missing (fallback provider, `PUBLIC_URL`, sender).

## Operating rules

- Migrations run at boot and are additive; every instance migrates on the
  same push. Never edit an applied migration.
- Replicas are allowed (SSE fans out through Postgres NOTIFY); pacing is per
  replica: divide the provider limit by the replica count.
- Scrape `GET /v1/metrics/prometheus` with the admin or read-only key as
  `bearer_token`.
- A push to `main` redeploys every instance within minutes: run
  `cargo test` and `docker build .` before pushing.
