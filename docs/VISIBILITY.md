# Visibility plan

How notifyd gets found in 2026, in the order that pays off first. Rules
below were read on the primary sources on 2026-09-05; re-read them before
acting, they move.

## What is already in the repository

| Asset | Where | Purpose |
|---|---|---|
| CI (check, test, image build) | `.github/workflows/ci.yml` | green badge, proof the compose works |
| GHCR multi-arch image on tag `v*` | `.github/workflows/release.yml` | `ghcr.io/rmzlb/notifyd`, amd64 + arm64, GitHub Release with notes |
| OCI + MCP labels | `Dockerfile` | links the package to the repo; proves ownership to the MCP registry |
| `server.json` | repo root | official MCP registry entry (`io.github.rmzlb/notifyd`), OCI package + `{host}` remote |
| MCP registry publish (manual) | `.github/workflows/mcp-registry.yml` | `mcp-publisher login github-oidc && publish` |
| Crate metadata | `Cargo.toml` (`readme`, `include`, categories) | `cargo publish` verified with `--dry-run` |
| Reproducible benchmarks | `docs/BENCHMARKS.md` | numbers only for notifyd, method, hardware, date, bias disclaimer |
| Agent Skills | `skills/` | `npx skills add rmzlb/notifyd`, skills.sh badge |
| `llms.txt` | `docs/llms.txt` | agent-readable API reference |

## Steps that need a human (rmzlb) — in order

1. **Repository metadata** (5 min). Topics, description, homepage, Discussions,
   social preview 1280×640 (< 1 MB):

   ```bash
   gh repo edit rmzlb/notifyd \
     --description "Agent-first notification service in Rust. Email, SMS, push, in-app inbox. One binary, Postgres only, MCP server built in." \
     --homepage "https://github.com/rmzlb/notifyd#readme" --enable-discussions \
     --add-topic mcp --add-topic mcp-server --add-topic model-context-protocol \
     --add-topic notifications --add-topic notification-service --add-topic email \
     --add-topic sms --add-topic push-notifications --add-topic rust --add-topic axum \
     --add-topic postgresql --add-topic self-hosted --add-topic selfhosted \
     --add-topic ai-agents --add-topic agent-skills --add-topic llms-txt --add-topic novu-alternative
   ```

2. **First tagged release** (10 min + 15 min of CI). Bump `version` in
   `Cargo.toml` and `server.json`, then:

   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

   Then open the package page (`github.com/rmzlb/notifyd/pkgs/container/notifyd`)
   → Package settings → **Change visibility → Public**. GHCR packages are
   private by default. Test `docker pull ghcr.io/rmzlb/notifyd:0.2.0` from a
   machine that has never built it. The release date starts the 4-month clock
   for awesome-selfhosted.

3. **Publish the crate** (10 min). `cargo login`, then `cargo publish` from a
   clean checkout of the tag. Reserves the name (free on 2026-09-05), gives
   `cargo install notifyd`, a download counter (awesome-rust criterion) and an
   automatic lib.rs listing. Never publish an empty crate to squat a name.

4. **MCP registry** (5 min). Actions → `mcp-registry` → Run workflow with the
   tag. Requires step 2 (public image). Then email
   `partnerships@github.com` to ask for inclusion in the GitHub MCP Registry
   (github.com/mcp), which is curated from the official registry.

5. **MCP lists** (20 min). PR to `punkpeye/awesome-mcp-servers`, section
   `💬 Communication`, one line, alphabetical:
   `- [rmzlb/notifyd](https://github.com/rmzlb/notifyd) 🎖️ 🦀 🏠 - Self-hosted notification service (email, SMS, push, in-app) with digest, jobs, suppressions and test sends as MCP tools.`
   Add `🤖🤖🤖` to the PR title for the fast lane. Also `mcpservers.org/submit`
   (free form; wong2's list takes no PRs) and Glama → Add Server.

6. **Show HN** (one shot, Tue–Thu, US morning). Title format is mandatory:
   `Show HN: Notifyd – single-binary notification service in Rust, Postgres only, MCP built in`.
   Body: what it is, why Postgres-only, the benchmark page, what is missing.
   Prerequisites: public image, `docker compose up` tested on a blank VM,
   author online for 6 hours to answer. Never ask for upvotes.

7. **r/rust the same day.** This Week in Rust no longer takes PRs for
   "Project/Tooling updates": the editors pick links that were posted on
   r/rust and upvoted. Read r/rust's self-promotion rule first (not readable
   from this server). Disclose any LLM-written text.

8. **selfh.st → "Project Launch"** (2 min form) the week of the release, plus
   a post on r/selfhosted (read its rules first). A technical article on
   dev.to ("a notification queue on Postgres alone, no Redis") relayed on
   Mastodon/Bluesky with `#rustlang` is the usual companion.

9. **January 2027** (4 months after the first release): PR to
   `awesome-selfhosted/awesome-selfhosted-data`, file `software/notifyd.yml`,
   tag `Communication - Custom Communication Systems`, `platforms: [Rust, Docker]`,
   `depends_3rdparty: true` (Resend/Twilio/FCM). Hand-written YAML: their
   rules ban LLM-generated contributions that miss the guidelines.

10. **awesome-rust** once the repo has > 50 stars or the crate > 2 000
    downloads (hard rule), section `Applications › Web`.

## Not now

- **Claude connectors directory**: submission portal is Team/Enterprise only,
  wants OAuth or a case-by-case agreement (`mcp-review@anthropic.com`) for
  self-hosted URLs. Revisit when there is a hosted demo.
- **Cursor directory, lobste.rs, Product Hunt**: low fit or invitation-only.
- **llms.txt directories** (`llmstxt.site`, `directory.llmstxt.cloud`): they
  index domains, not repository files. Needs a domain serving `/llms.txt`.

## Mistakes that cost more than they bring

- Submitting to awesome-selfhosted or awesome-rust before the criteria are
  met: a closed PR stays visible.
- Unmeasured numbers about competitors in the README or benchmarks.
- Astroturfing, upvote requests, multiple accounts, the same link on several
  subreddits the same day.
- A Show HN on a version whose `docker compose up` was not tested on a clean
  machine; a second Show HN for a minor release.
- API key in the MCP URL (`server.json` uses `headers[]` with `isSecret`).
- An MCP name outside `io.github.rmzlb/` with GitHub authentication.
