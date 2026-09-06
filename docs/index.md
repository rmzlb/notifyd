# notifyd

The notification service your agent can send through **and** run. Email,
SMS, WhatsApp, push, in-app inbox. One Rust binary, PostgreSQL only, no
dashboard: a digest endpoint and an MCP server instead.

- Repository and README: [github.com/rmzlb/notifyd](https://github.com/rmzlb/notifyd)
- Install: `docker pull ghcr.io/rmzlb/notifyd` · `cargo install notifyd` · `nix run github:rmzlb/notifyd`
- MCP registry: `io.github.rmzlb/notifyd`

## Writing

- [A notification queue on PostgreSQL alone: what `SKIP LOCKED` gives you, and what it does not](articles/postgres-queue-what-skip-locked-does-not-give-you.html) — state of the art, the six concerns a claim primitive leaves open, the design, measurements, limits, open questions. 2026-09-06.

## Documentation

- [Setup](SETUP.html) · [API reference](API.html) · [Agent operations](AGENT.html) · [Connectors](CONNECTORS.html)
- [Architecture](ARCHITECTURE.html) · [Benchmarks](BENCHMARKS.html) · [Deployments](DEPLOYMENTS.html)
- [llms.txt](llms.txt) — the API in plain text, for agents

<p><img src="assets/notifyd-explainer.gif" alt="60-second explainer" width="880"></p>
