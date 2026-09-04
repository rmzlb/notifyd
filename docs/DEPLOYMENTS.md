# Deployments — one notifyd per company

notifyd is deployed **once per company**, always from this repository, always
from `main`, through a Dokploy *Docker Compose* service that points at
`./docker-compose.yml`. Everything company-specific lives in the Dokploy
environment of that service; nothing company-specific lives in this repo.
No secret value is ever written here, only variable names.

## Why per company, not one shared instance

- Each company owns its data (one Postgres), its Resend team, its sender
  identity, its API keys. Deleting a customer is deleting a stack.
- A marketing burst of one company never delays another company's
  transactional emails: the worker queue is per instance.
- A push to `main` rebuilds every instance from the same commit, and the
  pinned Dockerfile makes the resulting binaries identical. `/v1/health`
  exposes `commit` and `built_at_epoch` so drift between instances is visible.

## Inventory

| Company | Server (Dokploy alias) | Dokploy service | Public URL | Notes |
|---|---|---|---|---|
| sqare + helmai (+ craie project) | square | compose `novu/notifyd-stack` | `notifyd.ctrlnz.com` | Historical shared instance. Global sender is Helmai. The `craie` project has no `from_email`: fix or migrate CRAIE to its own stack. An orphan Application `notifyd/notifyd` (2026-03) on the same server still auto-deploys from this repo and serves nothing: delete it. |
| Philoé | philoe | compose `notifyd` (to create) | internal `http://notifyd:3400`; public URL only for the admin in-app inbox (SSE from the browser) | See "Creating an instance". |
| CRAIE | leftcurve | none yet | — | Currently uses the shared instance over the public internet. |

Keep this table current: it is the only place that says which instance
serves which company.

## Creating an instance (Dokploy)

1. Project → *Create Service* → **Compose**. Provider: GitHub, repository
   `rmzlb/notifyd`, branch `main`, compose path `./docker-compose.yml`,
   Autodeploy **on**.
2. Environment (names; generate secrets with `openssl rand -hex 32`):
   `POSTGRES_PASSWORD`, `JWT_SECRET`, `ADMIN_API_KEY`, `RESEND_API_KEY` (the
   company's Resend team), `EMAIL_FROM`, `EMAIL_FROM_NAME`, `CORS_ORIGINS`
   (the company's back-office origin), `PORT=3400`, `RUST_LOG=notifyd=info`.
   Optional: `RESEND_WEBHOOK_SECRET` (only if this instance, and not the
   application, is the consumer of the Resend webhooks), push/SMS connector
   variables.
3. Deploy. `docker compose` attaches the container to `dokploy-network` under
   the alias `notifyd`, so applications on the same server reach it at
   `http://notifyd:3400` without leaving the host. Add a public domain in
   Dokploy only if browsers must reach it (in-app inbox over SSE).
4. Create the company project, from the server (`docker exec` into the
   container, or any host that can reach the instance):

   ```sh
   curl -s -X POST "$NOTIFYD_URL/v1/admin/projects" \
     -H "x-api-key: $ADMIN_API_KEY" -H 'content-type: application/json' \
     -d '{"id":"<company>","name":"<Company>","channels":["email","in_app"],
          "from_email":"<sender@company-domain>","from_name":"<Company>"}'
   ```

   The response carries the project API key: it goes into the application's
   `NOTIFYD_API_KEY`, nowhere else.
5. Check: `GET /v1/health` returns `status: ok`, the current `commit`, and a
   fresh `built_at_epoch`; `GET /v1/admin/projects` lists the project with its
   `from_email`.

## Rules that keep N instances safe

- **Migrations are additive only.** Every instance runs `sqlx::migrate!` at
  boot on the same push; a destructive migration breaks every company at the
  same minute. Add columns and tables, never drop or rename in the same
  release as the code that stops using them.
- **A project without `from_email` sends as the instance default.** Always
  set `from_email` and `from_name` when creating a project.
- **Pins move on purpose.** The Dockerfile pins images by digest; refresh them
  together with `Cargo.lock`, never implicitly.
- **`replicas: 1`.** The in-app inbox uses an in-memory broadcast; scale
  vertically or port `src/sse.rs` to Postgres `LISTEN/NOTIFY` first.
