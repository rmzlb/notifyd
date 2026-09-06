---
title: "A notification queue on PostgreSQL alone: what SKIP LOCKED gives you, and what it does not"
author: Ramzi Laieb (rmzlb)
date: 2026-09-06
description: State of the art of Postgres-backed queues, the gap between a claim primitive and a delivery engine for email and SMS, the design notifyd uses to close it, measured results, limits, and open questions.
---

# A notification queue on PostgreSQL alone: what `SKIP LOCKED` gives you, and what it does not

*Ramzi Laieb, 2026-09-06. Applies to notifyd 0.2.1. Parts of the code and of
this text were drafted with an AI coding assistant and reviewed by the author.
Every citation was fetched from its primary source on 2026-09-06; two that
could not be fetched are marked as such.*

## Abstract

`SELECT … FOR UPDATE SKIP LOCKED` has made PostgreSQL a credible job queue
since 9.5 (2016) and a default one in several ecosystems (Rails 8's Solid
Queue, Oban, River, Graphile Worker, pg-boss). Notification delivery, however,
is not a generic job: the bottleneck is not the database but a third-party
provider with an account-wide quota, whose `429` answers must be interpreted,
whose failures must be classified, and whose limits must be shared between a
password reset and a 5 000-recipient campaign. This article surveys what the
Postgres-queue literature and the existing queues provide, identifies six
concerns that the claim primitive leaves open, describes how notifyd (a
single Rust binary, Postgres as its only dependency) addresses them, and
reports measurements on commodity hardware: an unoptimised implementation
drained 516 jobs/s; batching the claim, the context lookups and the
finalisation, and removing a per-job task, brought it to 3 537 jobs/s on the
same 8-vCPU host with 23 MB of resident memory. We then list what these
numbers do not show, and what remains open.

## 1. Problem

A product sends two kinds of notifications through the same providers:
transactional messages (order shipped, password reset) whose value decays in
seconds, and campaigns whose value is indifferent to a delay of an hour. Both
go out through providers that meter by account: Resend's default limit is
10 requests per second per team with a 100-message batch endpoint [R1, R2];
Amazon SES exposes a per-second *sending rate* that "you can exceed for short
bursts, but not for sustained periods" [R3]; Postmark caps a batch at 500
messages [R4]. A provider answers excess with `429 Too Many Requests`, which
RFC 6585 allows to carry a `Retry-After` header [L3]. Since February 2024,
bulk senders to Gmail and Yahoo must also keep the reported spam rate under
0.3 % and honour one-click unsubscribe within two days [L5, L6, L7].

The engineering question is therefore not "can Postgres hold a queue" (it
can) but "what must sit between the queue and the provider so that a
campaign never starves a password reset, a `429` never burns a retry, and an
operator, human or agent, can see why something did not leave".

## 2. State of the art

### 2.1 The primitive and its caveats

`SKIP LOCKED` landed in PostgreSQL 9.5.0 on 2016-01-07, in the same release as
`INSERT … ON CONFLICT` [P1]. The manual is explicit about its intended use and
its cost: "Skipping locked rows provides an inconsistent view of the data, so
this is not suitable for general purpose work, but can be used to avoid lock
contention with multiple consumers accessing a queue-like table" [P2]. Two
caveats in the same page matter for a queue: the table-level `ROW SHARE`
lock is still taken, and with `LIMIT`, "locking stops once enough rows have
been returned to satisfy the limit" [P2]. The alternative, advisory locks, is
"faster, avoid[s] table bloat" but carries its own `LIMIT` hazard, which the
manual illustrates with a query it labels `-- danger!` [P3].

The operational caveat is dead tuples. A queue row is inserted, updated to
`processing`, then to `sent` or `retry`: three versions per job. Brandur
Leach documented in 2015 how a long-running transaction elsewhere left
"247311 dead row versions [that] cannot be removed yet" and pushed the lock
query "from < 0.01 seconds to 0.1 s and above" [P5]. Crunchy Data's 2021
write-up of the `SKIP LOCKED` + `DELETE … RETURNING` pattern ends on the same
advice: monitor bloat, tune autovacuum, possibly rotate the table [P6].
Hatchet's 2026 survival guide is blunter: "If autovacuum can't keep up …
you'll get into a very unhealthy state, very quickly" [P7]. The manual itself
notes that "some installations with extremely high update rates vacuum their
busiest tables as often as once every few minutes" [P4]. PostgreSQL 17 made
vacuum's dead-tuple storage up to 20× smaller [P8]; PostgreSQL 18 added
`autovacuum_vacuum_max_threshold`, a fixed dead-tuple trigger that no longer
scales with table size, plus asynchronous I/O covering vacuum and B-tree skip
scans [P9]. River's 2026 "concurrent repack" describes the remaining gap:
vacuum "marks space as reusable … but never fully reclaims it" [P10].

### 2.2 Postgres-backed job queues

Table 1 summarises the queues whose source or documentation we read. All but
two use `SKIP LOCKED`; most add `LISTEN/NOTIFY` to wake workers.

| Queue | Language | Claim | Published throughput | Priority | Outbound rate limiting |
|---|---|---|---|---|---|
| pg-boss 12 [Q1] | Node | `SKIP LOCKED` + NOTIFY | none | yes | queue storage policies |
| Graphile Worker 0.17 [Q2] | Node | `SKIP LOCKED` + NOTIFY | ~183 000 jobs/s batched, ~15 600 unbatched; 200 000 trivial jobs, 4 processes × 24, i9-14900K, DB on the same machine | yes | no |
| Oban 2.24 [Q3] | Elixir | `SKIP LOCKED`, `ORDER BY priority, scheduled_at, id` | formula only: `(1000 / cooldown) × limit` per queue | 10 levels | Pro only ("Smart Engine") |
| River 0.47 [Q4] | Go | `SKIP LOCKED`, `ORDER BY priority, scheduled_at, id` | ~46 000 jobs/s, 1 M no-op jobs, 2 000 goroutines, M2 MacBook Air | 1–4 | Pro concurrency limits; global rate limiting "groundwork" (2025) |
| Solid Queue 1.7 [Q5, Q6] | Ruby | `SKIP LOCKED` "if available" | HEY: ~20 M jobs/day; ~1 300 polling queries/s at 110 µs | integer | concurrency controls, no time-based limit |
| Que [Q7] | Ruby | advisory locks, NOTIFY | none | integer | no |
| PGMQ [Q8] | SQL extension | `SKIP LOCKED` + visibility timeout | none | no (FIFO) | no |
| Procrastinate [Q9] | Python | `SKIP LOCKED`, `LIMIT 1` | none published | yes | no |
| apalis-postgres 1.0-rc [Q10] | Rust | `SKIP LOCKED`, `ORDER BY priority DESC, run_at` | none | yes | tower layers, not queue-native |
| sqlxmq [Q11] | Rust | `UPDATE … FROM (SELECT … LIMIT)` re-checked, NOTIFY | none | no | concurrency only |
| pgqueuer [Q12] | Python | `SKIP LOCKED` + NOTIFY | none | yes | per-entrypoint limits |

*Table 1. Only Graphile Worker and River publish a jobs/s figure with the
hardware. None of the surveyed queues ships a per-channel token bucket or a
notion of "the provider said 429" inside the queue: rate limiting, where it
exists, bounds the consumer's own concurrency, not a third party's quota.*

Two figures from Table 1 frame our results. Graphile Worker's ~183 000 jobs/s
and River's ~46 000 jobs/s are for *no-op* jobs; the authors say so, and River
adds that "benchmarking is a highly imperfect science" [Q4]. A notification
job is not a no-op: it resolves a sender identity, checks suppressions and
preferences, renders a template, builds an RFC 5322 message with
unsubscribe headers, and calls a provider. Section 5 measures that path with
the provider call stubbed, which is the right comparison point for the
engine, and states what it leaves out.

### 2.3 Notification platforms and the provider limit

The notification platforms we could read handle the provider limit
differently from what a queue reader might expect. Novu's documentation
states that a channel step "makes a single call to the provider. There is no
automatic retry, no backoff, and no ceiling, because there is no second
attempt", and that "a `429 Too Many Requests` … from a provider is handled the
same way as a `400 Bad Request`" [N1]; the worker source confirms that the
send is wrapped in a `try/catch` that records `PROVIDER_ERROR` without
inspecting the status code [N2]. Knock, Courier and SuprSend each document a
"throttle" step, but all three define it as a per-recipient anti-flood
control ("limit the number of times a workflow is executed for a recipient
within a given window" [N3]; "how many … messages a user or group receives
within a set timeframe" [N4]; "rate limit workflow executions per user" [N5]),
not as pacing against the provider. Their API rate limits are documented on
the inbound side [N3]. We found no public documentation of provider-side
`429` pacing at any of the four.

### 2.4 Delivery-engine literature

The mechanisms we use are old and documented. Token buckets bound rate and
burst [L1]; RFC 2697 formalises a two-rate variant [L2]. Exponential backoff
with jitter is analysed by Brooker (2015), whose "full jitter" and
"decorrelated jitter" formulas are the usual references [L4]. `Retry-After`
is specified in RFC 9110 §10.2.3 and attached to `429` by RFC 6585 §4 [L3].
One-click unsubscribe is RFC 8058 [L5]; Gmail's and Yahoo's 2024 requirements
make it mandatory above 5 000 messages a day and set the 0.3 % spam-rate
ceiling [L6, L7]. Staging jobs in the same transaction as the business write,
so that a crash between commit and enqueue cannot lose them, is Leach's
"transactionally staged job drain" [D2]; the broader "use Postgres, spend
your innovation tokens elsewhere" argument is McKinley's [D1] and Hunt's [D3].

## 3. What the primitive leaves open

Reading Sections 2.1 to 2.3 together, six concerns are not addressed by the
claim statement, and are only partly addressed by the queues built on it.

1. **Fairness under a shared external quota.** Priority ordering (Oban,
   River, Solid Queue) decides which job is claimed first. It does not
   decide who gets the provider's 10 requests per second once a campaign
   has been enqueued: without pacing, the first worker to claim 500 bulk
   jobs will spend the quota, and the password reset claimed a millisecond
   later will meet a `429`.
2. **Interpreting the provider's answer.** A `429` is not a failure of the
   job. Counting it as an attempt (Novu counts it as terminal) means a
   healthy campaign can exhaust its retries against a healthy provider.
3. **Per-job overhead.** Table 1's throughputs are for jobs that do nothing.
   A notification job's fixed costs are database round trips: sender
   lookup, suppression check, preference check, template fetch, webhook
   fan-out, finalisation. At one round trip each, the engine is bounded by
   Postgres latency, not by the provider.
4. **Batch semantics of providers.** Resend accepts 100 messages per call
   and rejects the whole batch on a validation error [R2]; SMTP and SMS
   providers take one message per call. The queue must batch where the
   provider batches, and fall back per item where a batch is refused.
5. **Failure classification.** A bounce, a `422 unverified sender`, a
   `503`, a network timeout and a `429` require five different reactions
   (suppress, fail fast, retry, retry, pause). A queue that exposes one
   `Err` forces the caller to encode this, and most callers do not.
6. **Observability for the person on call.** Dead tuples, paused channels,
   retry storms and bounce rates are visible in Postgres, but nobody on
   call runs `SELECT count(*) … GROUP BY status` at 3 a.m. The queues in
   Table 1 expose metrics; the platforms in 2.3 expose dashboards. Neither
   says what to do.

## 4. Design

notifyd is one Rust binary (axum, sqlx, tokio) with PostgreSQL as its only
dependency. This section describes the parts of it that answer Section 3.
All SQL below is quoted from `src/worker.rs` and `src/api/send.rs` at 0.2.1.

### 4.1 Claim

Jobs are rows. A worker claims a batch in a transaction, orders by priority
then by schedule, skips channels that a provider has asked to pause, and marks
the batch in a second statement:

```sql
SELECT … FROM jobs
WHERE status IN ('pending', 'retry')
  AND scheduled_at <= $1
  AND (next_retry_at IS NULL OR next_retry_at <= $1)
  AND NOT (channel = ANY($3))          -- channels paused after a 429
ORDER BY priority ASC, scheduled_at ASC
LIMIT $2
FOR UPDATE SKIP LOCKED;

UPDATE jobs SET status = 'processing', attempts = attempts + 1, claimed_at = now()
WHERE id = ANY($1);
```

A partial index `ON jobs (priority, scheduled_at) WHERE status IN ('pending',
'retry')` keeps the claim cheap as the table fills with `sent` rows. Priority
is a 0–100 integer; the API accepts `critical` (10), `high` (30), `normal`
(50), `low` (70), `bulk` (80) or a number, and `POST /v1/batch` defaults to
`bulk`, as does any send tagged `campaign`, `marketing` or `newsletter`.

### 4.2 Pacing and the meaning of a `429`

A token bucket per channel (`EMAIL_RATE_PER_SEC`, `SMS_RATE_PER_SEC`) bounds
outbound calls per replica [L1]. When a provider answers `429`, the worker
(a) tries the fallback provider if one is configured, then (b) pauses the
*channel* for `Retry-After` when the provider sent one, or a configured
default otherwise, and (c) re-queues the job **without consuming an
attempt**:

```sql
UPDATE jobs SET status = 'retry', attempts = GREATEST(attempts - 1, 0),
                error = $2, next_retry_at = $3 WHERE id = $1;
```

Priorities do not bypass the pause. The provider's limit is per account, so a
`critical` message sent during the pause would be refused as well. What the
design guarantees is order on resume: the claim statement puts `critical`
ahead of `bulk`, so the password reset is in the first batch after
`Retry-After` elapses, whatever the campaign backlog. Other channels are not
affected by an email pause.

### 4.3 Failure classification and retries

Connectors return a typed error: `RateLimited { retry_after }`, `Transient`,
`Permanent`, `Suppressed`. Only `Transient` and `RateLimited` trigger the
failover breaker; `Permanent` (4xx other than 429, SQLSTATE 23xxx on
in-app inserts) fails immediately; `Suppressed` records the outcome without
calling the provider. Transient failures follow a fixed schedule of 30 s,
2 min, 10 min, 30 min, 2 h with ±20 % jitter [L4], five attempts by default.
A reaper re-queues jobs left in `processing` for more than ten minutes, so a
worker that dies mid-batch loses at most that.

### 4.4 Batching where the provider batches

Each connector declares `batch_max()`: 100 for Resend (its API's ceiling
[R2]), 1 for SMTP, Twilio and Telnyx. The worker chunks the claimed batch
accordingly. A batch refused with a 4xx is retried item by item, so one bad
address does not fail 99 good ones. Successful items are finalised in one
statement:

```sql
UPDATE jobs SET status = 'sent', sent_at = now(), error = NULL,
                provider = r.provider, provider_message_id = r.mid
FROM unnest($1::uuid[], $2::text[], $3::text[]) AS r(id, provider, mid)
WHERE jobs.id = r.id;
```

### 4.5 Removing per-job round trips

Before the change measured in Section 5, each job performed its own sender
lookup, suppression check, preference check and spawned a webhook task that
created an HTTP client and queried the project's webhooks. After it, the
worker loads senders, suppressions, preferences and the set of projects that
have webhooks once per claimed batch; if that prefetch fails, it falls back
to the per-job query rather than skipping the check (a suppression must
never be bypassed because a cache failed). `POST /v1/batch` inserts its N
jobs with one `INSERT … SELECT FROM unnest(…)`, with the idempotency
conflict handled by a partial unique index on `(project_id, idempotency_key)
WHERE status NOT IN ('failed', 'cancelled')`.

### 4.6 Governance and observability

Marketing email carries RFC 8058 headers pointing at an HMAC-signed
unsubscribe URL [L5]; suppressions have a scope (`all` or `marketing`) so a
customer who leaves the newsletter still receives their invoice. Send windows
are evaluated in the recipient's timezone. `GET /v1/admin/digest` ranks
findings (paused channel, bounce rate above 2 % or 5 %, oldest waiting job,
failed jobs with their top cause, missing fallback provider) and attaches to
each one the action an operator would take. The same operations are exposed
as MCP tools with `readOnlyHint` / `destructiveHint` annotations, and a
read-only key restricts an agent to the reporting subset. This is the answer
to concern 6: not a dashboard, but a ranked list with actions that a human
or an agent can execute.

## 5. Evaluation

### 5.1 Method

| | |
|---|---|
| Host | 8 vCPU Arm Neoverse-V2, 30 GB RAM, Ubuntu, Docker |
| PostgreSQL | 16, default `postgresql.conf`, one container on the same host |
| notifyd | 0.2.x release build; `WORKER_BATCH_SIZE=500`, `WORKER_POLL_INTERVAL_MS=100`, `DATABASE_MAX_CONNECTIONS=20`, `EMAIL_RATE_PER_SEC=0` (pacer disabled) |
| Provider | `EMAIL_PROVIDER=log`, a no-op connector with `batch_max = 100`, i.e. the Resend code path minus the network |
| Load | one project, unlimited inbound rate; 100 000 email jobs enqueued through `POST /v1/batch` in calls of 5 000, all `scheduled_at` in the future, released by one `UPDATE`, drained by one worker |
| Measurement | wall-clock from release to `count(*) WHERE status IN ('pending','processing','retry') = 0`; RSS sampled every 250 ms with `ps`; `sent_at - scheduled_at` percentiles from the table |

The commands are in `docs/BENCHMARKS.md`. Everything runs on one machine,
which flatters latency and penalises nothing else; it is the setup a small
company actually deploys.

### 5.2 Results

| Path | Before (0.2.0-pre) | After (3423bc7) |
|---|---:|---:|
| `POST /v1/batch`, 5 000 recipients per call | 719 jobs/s | 44 546 jobs/s |
| Drain, provider batching 100 | 516 jobs/s | 3 537 jobs/s |
| Drain, provider batching 1 (SMTP, SMS) | — | 640 jobs/s |
| RSS at idle / peak while draining 100 000 jobs | 13 MB / 22 MB | 13 MB / 23 MB |
| `jobs` table + indexes after 100 000 sent jobs | | 84 MB (0.85 kB/job, body included) |
| `GET /v1/admin/digest` over 100 000 jobs | | 70 ms |
| `POST /v1/send`, 8 concurrent clients | | 1 867 req/s, p50 3.9 ms |

*Table 2. Same host, same Postgres, same load. "Before" is the per-job
implementation; "after" is Section 4.4–4.5.*

### 5.3 Interpretation

The 6.9× drain improvement did not come from `SKIP LOCKED`, which was already
there; it came from removing per-job round trips (concern 3) and batching
finalisation (concern 4). With a non-batching provider the same engine caps
near 640 jobs/s: each message becomes its own provider call and its own
finalisation `UPDATE`, which is the shape SMTP and SMS impose in production
anyway, where the provider's quota, not the engine, is the ceiling.

Against Table 1, 3 537 jobs/s is an order of magnitude below Graphile Worker
and River, and it should be: a no-op job has none of the fixed costs listed
in Section 3, and our poll interval (100 ms, no `LISTEN/NOTIFY`) bounds
minimum latency. For the target workload the relevant comparison is the
provider: at Resend's 10 requests/s × 100 recipients, 100 000 emails leave in
roughly 100 s if the provider allows the burst and around 8–9 minutes at a
paced 2 requests/s; the engine spends most of that time waiting on the token
bucket, at 23 MB of memory.

## 6. Limits and what is not covered

- **Provider stubbed.** No network, no TLS, no provider latency, no real
  `429`. The pacing and pause paths are unit-tested, not benchmarked.
- **One worker, one replica.** We did not measure lock contention with
  several workers claiming from the same table, which is the case the
  manual's `SKIP LOCKED` paragraph is about [P2]. Pacing is per replica; two
  replicas share nothing and must each be configured with half the quota.
- **Sustained churn.** Runs lasted under a minute. Dead-tuple accumulation,
  autovacuum behaviour and index bloat over days of traffic (Section 2.1)
  were not measured. The partial claim index limits the read cost of dead
  rows but does not remove them.
- **Default Postgres.** No tuning, no partitioning, no table rotation.
- **Polling, not `LISTEN/NOTIFY`.** Minimum scheduling latency is the poll
  interval; the in-app SSE path uses `NOTIFY`, the worker does not.
- **At-least-once.** A worker crash after the provider accepted a batch but
  before `finalize_sent_batch` commits results in a resend after the reaper
  fires; the provider's idempotency, where it exists, is not used.
- **Competitors not measured.** We did not run Novu or any queue from
  Table 1 through this protocol; Table 1 reports their own published
  figures, on their own hardware and workloads.
- **No formal model.** Fairness (Section 3, concern 1) is argued from the
  claim order, not proven; a starvation analysis under adversarial bulk
  load is future work.

## 7. Open questions

1. **Wake-up.** Adding `LISTEN/NOTIFY` to the worker, as most queues in
   Table 1 do, would cut minimum latency from the poll interval to
   milliseconds; the cost is one more connection per replica and a fallback
   poll for missed notifications.
2. **Churn.** Which of PGMQ's partitioning [Q8], River's concurrent repack
   [P10], or PostgreSQL 18's `autovacuum_vacuum_max_threshold` [P9] is the
   right default for a queue table that sees 3 × N tuple versions per N
   jobs is an empirical question we have not answered.
3. **Global pacing.** Per-replica token buckets do not add up to the
   provider's quota under scaling. A shared bucket in Postgres (one row per
   channel, `UPDATE … RETURNING`) costs a round trip per batch and would be
   exact; River's Pro roadmap names the same problem [Q4].
4. **Adaptive pacing.** Resend, SendGrid and Postmark return remaining-quota
   headers [R1, R4, R5]. Reading them would let the bucket follow the
   provider instead of a static rate.
5. **Fairness across projects.** One instance per company sidesteps
   multi-tenant fairness; Hatchet's work on fair multi-tenant queues in
   Postgres [D5] is the reference if that assumption changes.
6. **Exactly-once toward the provider.** Providers with idempotency keys
   could close the at-least-once window in Section 6; most email APIs do
   not offer one.
7. **Skip scans.** PostgreSQL 18's B-tree skip scan [P9] may make a single
   `(status, priority, scheduled_at)` index serve both the claim and the
   operator queries; we have not measured it.

## 8. Reproducibility

Code: `github.com/rmzlb/notifyd`, tag `v0.2.1`, MIT. Benchmark protocol and
commands: `docs/BENCHMARKS.md`. Unit tests: `cargo test` (65 tests, no
database required). The numbers in Table 2 were produced on 2026-09-05 and
2026-09-06 on the host described in 5.1; we will link any independent
measurement, on any hardware, that follows the protocol.

## References

**PostgreSQL**

- [P1] PostgreSQL 9.5.0 release notes, 2016-01-07. https://www.postgresql.org/docs/release/9.5.0/
- [P2] PostgreSQL manual, `SELECT`, "The Locking Clause". https://www.postgresql.org/docs/current/sql-select.html
- [P3] PostgreSQL manual, "Explicit Locking", §Advisory Locks. https://www.postgresql.org/docs/current/explicit-locking.html
- [P4] PostgreSQL manual, "Routine Vacuuming". https://www.postgresql.org/docs/current/routine-vacuuming.html
- [P5] B. Leach, "Postgres Job Queues & Failure By MVCC", 2015-05-18. https://brandur.org/postgres-queues
- [P6] D. Christensen, "Devious SQL: Message Queuing Using Native PostgreSQL", Crunchy Data, 2021-09-01. https://www.crunchydata.com/blog/message-queuing-using-native-postgresql
- [P7] A. Belanger, "The startup's Postgres survival guide", Hatchet, 2026-07-22. https://hatchet.run/blog/postgres-survival-guide
- [P8] PostgreSQL 17 release notes, 2024-09-26. https://www.postgresql.org/docs/17/release-17.html
- [P9] PostgreSQL 18 release notes, 2025-09-25. https://www.postgresql.org/docs/18/release-18.html
- [P10] B. Leach, "Concurrent repack: Vacuum full without the pain", River, 2026-04-27. https://riverqueue.com/blog/repack-concurrently
- [P11] C. Ringer, "What is SELECT SKIP LOCKED for in PostgreSQL 9.5?", 2ndQuadrant, 2016. *Original URL no longer resolves; not fetched.*

**Queues**

- [Q1] pg-boss. https://github.com/timgit/pg-boss
- [Q2] Graphile Worker, "Performance". https://worker.graphile.org/docs/performance
- [Q3] Oban. https://oban.hexdocs.pm/Oban.html ; claim statement in `lib/oban/engines/basic.ex`.
- [Q4] River, "Benchmarks". https://riverqueue.com/docs/benchmarks ; B. Gentry, "Concurrency limits", 2025-04-14. https://riverqueue.com/blog/concurrency-limits
- [Q5] R. Gutiérrez, "Introducing Solid Queue", 37signals, 2023-12-18. https://dev.37signals.com/introducing-solid-queue/
- [Q6] D. H. Hansson, "Rails 8.0: No PaaS Required", 2024-11-07. https://rubyonrails.org/2024/11/7/rails-8-no-paas-required
- [Q7] Que. https://github.com/que-rb/que
- [Q8] PGMQ. https://github.com/pgmq/pgmq
- [Q9] Procrastinate, "Discussions". https://procrastinate.readthedocs.io/en/stable/discussions.html
- [Q10] apalis-postgres. https://docs.rs/apalis-postgres
- [Q11] sqlxmq. https://github.com/Diggsey/sqlxmq
- [Q12] pgqueuer. https://github.com/janbjorge/pgqueuer

**Notification platforms and providers**

- [N1] Novu, "Delivery retries". https://docs.novu.co/platform/developer/delivery-retries
- [N2] Novu, `apps/worker/src/app/workflow/usecases/send-message/send-message-email.usecase.ts`, branch `next`, read 2026-09-06.
- [N3] Knock, "Throttle function" and "Rate limits". https://docs.knock.app/designing-workflows/throttle-function ; https://docs.knock.app/api-reference/overview/rate-limits
- [N4] Courier, "Throttle". https://www.courier.com/docs/platform/automations/throttle
- [N5] SuprSend, "Throttle". https://docs.suprsend.com/docs/throttle
- [R1] Resend, "Rate limit". https://resend.com/docs/api-reference/rate-limit
- [R2] Resend, "Send batch emails". https://resend.com/docs/api-reference/emails/send-batch-emails
- [R3] Amazon SES, "Managing your sending quotas". https://docs.aws.amazon.com/ses/latest/dg/manage-sending-quotas.html
- [R4] Postmark, "API overview". https://postmarkapp.com/developer/api/overview
- [R5] SendGrid, "Rate limits". https://www.twilio.com/docs/sendgrid/api-reference/how-to-use-the-sendgrid-v3-api/rate-limits

**Mechanisms and requirements**

- [L1] Token bucket, Wikipedia; J. Turner, "New directions in communications", IEEE Communications Magazine 24(10), 1986. https://en.wikipedia.org/wiki/Token_bucket
- [L2] RFC 2697, "A Single Rate Three Color Marker", 1999. https://datatracker.ietf.org/doc/html/rfc2697
- [L3] RFC 9110 §10.2.3 "Retry-After"; RFC 6585 §4 "429 Too Many Requests". https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after ; https://www.rfc-editor.org/rfc/rfc6585.html#section-4
- [L4] M. Brooker, "Exponential Backoff And Jitter", AWS Architecture Blog, 2015-03-04. https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/
- [L5] RFC 8058, "Signaling One-Click Functionality for List Email Headers", 2017. https://www.rfc-editor.org/rfc/rfc8058.html
- [L6] Google, "Email sender guidelines"; N. Kumaran, "New Gmail protections for a safer, less spammy inbox", 2023-10-03. https://support.google.com/a/answer/81126 ; https://blog.google/products/gmail/gmail-security-authentication-spam-protection/
- [L7] Yahoo, "Sender best practices". https://senders.yahooinc.com/best-practices/

**Discourse**

- [D1] D. McKinley, "Choose Boring Technology", 2015-03-30. https://mcfunley.com/choose-boring-technology
- [D2] B. Leach, "Transactionally Staged Job Drains in Postgres", 2017-09-20. https://brandur.org/job-drain
- [D3] P. Hunt, "Postgres: a Better Message Queue than Kafka?", Dagster, 2022-10-04. https://dagster.io/blog/skip-kafka-use-postgres-message-queue
- [D4] A. Ronacher, "Absurd Workflows: Durable Execution With Just Postgres", 2025-11-03. https://lucumr.pocoo.org/2025/11/3/absurd-workflows/
- [D5] A. Belanger, "An unfair advantage: multi-tenant queues in Postgres", Hatchet, 2024-04-18. https://hatchet.run/blog/multi-tenant-queues
