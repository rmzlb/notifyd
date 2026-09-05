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

## Inventory (verified 2026-09-05)

| Company | Server (Dokploy alias) | Dokploy service | Public URL | Notes |
|---|---|---|---|---|
| CRAIE | CRAIE's own server (own Dokploy) | compose from this repo, autodeploy `main` | `notifyd.craie.ctrlnz.com` | Dedicated instance, redeploys within ~5 min of a push to `main`. |
| sqare + helmai | square | compose `novu/notifyd-stack` | `notifyd.ctrlnz.com` | Shared by two companies. Global sender is Helmai; the `sqare` project has its own `from_email`. Contains a **stale `craie` project** (created 2026-06, no key hash, zero jobs): delete it. An orphan Application `notifyd/notifyd` (2026-03) exists on the same server; its autodeploy is now off, delete it. |
| Philoé | philoe | compose `notifyd` (git source, autodeploy `main` via a GitHub webhook on this repo) | internal `http://notifyd:3400`; public `https://api-os.philoeparis.com/notifyd` (path route on the API host, `stripPath`) for the admin in-app inbox | Created 2026-09-05. Path route instead of a dedicated hostname because no Cloudflare DNS token was available; switch to `notifyd-os.philoeparis.com` when one exists. Project `philoe`, `from_email` in `philoeparis.fr`. |

All instances use the same Resend team today (every company domain is
verified there). A per-company Resend team would need a per-instance
`RESEND_API_KEY`, which this layout already allows.

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

## Two things every new instance needs

- **Delete the seeded `craie` project.** Migration `005_craie_project.sql` inserts a
  `craie` project (random plaintext key, no hash, so it cannot authenticate) and
  CRAIE templates into every fresh database. On any instance that is not CRAIE's,
  `DELETE /v1/admin/projects/craie` right after the first boot. The `craie`
  project seen on the square instance is this seed, not a CRAIE integration.
- **Autodeploy without the GitHub App.** A compose with a `git` source needs a
  push webhook on this repository pointing at
  `<dokploy>/api/deploy/compose/<refreshToken>` (the token is on the compose
  record). Dokploy only deploys when the pushed branch matches the configured one.

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
