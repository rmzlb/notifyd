//! Operations an operator, human or agent, performs on a running notifyd:
//! read a digest of the instance, search jobs, retry or cancel one, manage
//! suppressions, adjust a project. One implementation, exposed twice: as
//! REST admin endpoints (`src/api/admin_ops.rs`) and as MCP tools
//! (`src/mcp.rs`). Nothing here knows about HTTP.

use crate::AppState;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

// ─── Digest ─────────────────────────────────────────────────────────────────

/// Windows the digest understands. Anything else is rejected so an agent
/// cannot ask for "last 3 years" and stall the database.
pub fn parse_window(raw: Option<&str>) -> Result<Duration> {
    match raw.unwrap_or("24h") {
        "1h" => Ok(Duration::hours(1)),
        "6h" => Ok(Duration::hours(6)),
        "24h" | "1d" => Ok(Duration::hours(24)),
        "7d" => Ok(Duration::days(7)),
        "30d" => Ok(Duration::days(30)),
        other => Err(anyhow!(
            "window must be 1h, 6h, 24h, 7d or 30d (got {other})"
        )),
    }
}

#[derive(Debug, Serialize)]
pub struct Digest {
    pub generated_at: DateTime<Utc>,
    pub window: String,
    pub instance: InstanceInfo,
    /// What deserves attention, most severe first. Empty means "all quiet".
    pub findings: Vec<Finding>,
    pub queue: QueueState,
    pub outcomes: Vec<OutcomeRow>,
    pub failures: Vec<FailureGroup>,
    pub retries_waiting: Vec<RetryRow>,
    pub latency: Vec<LatencyRow>,
    pub deliverability: Deliverability,
    pub projects: Vec<ProjectRow>,
    pub workflows_active: i64,
}

#[derive(Debug, Serialize)]
pub struct InstanceInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub built_at_epoch: u64,
    pub uptime_seconds: u64,
    pub email_provider: Option<String>,
    pub sms_provider: Option<String>,
    pub whatsapp_provider: Option<String>,
    pub paused_lanes: Vec<String>,
    pub email_fallback_provider: Option<String>,
    /// Seconds before the primary email provider is tried again, when the
    /// failover breaker is open.
    pub email_primary_resting_seconds: Option<u64>,
    pub email_failovers_since_boot: u64,
    /// `PUBLIC_URL`, base of the unsubscribe links on bulk email.
    pub public_url: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Finding {
    /// `critical` | `warning` | `info`
    pub severity: &'static str,
    pub message: String,
    /// What an operator would do about it.
    pub action: String,
}

#[derive(Debug, Serialize, Default)]
pub struct QueueState {
    pub pending: i64,
    pub retry: i64,
    pub processing: i64,
    pub oldest_waiting_seconds: Vec<BandAge>,
}

#[derive(Debug, Serialize)]
pub struct BandAge {
    pub band: String,
    pub seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct OutcomeRow {
    pub channel: String,
    pub provider: String,
    pub sent: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize)]
pub struct FailureGroup {
    pub channel: String,
    pub reason: String,
    pub count: i64,
    pub sample_job_id: Uuid,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RetryRow {
    pub channel: String,
    pub attempts: i32,
    pub count: i64,
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct LatencyRow {
    pub channel: String,
    pub sent: i64,
    pub p50_seconds: f64,
    pub p95_seconds: f64,
}

#[derive(Debug, Serialize, Default)]
pub struct Deliverability {
    pub delivered: i64,
    pub bounced: i64,
    pub complained: i64,
    pub suppressions_added: i64,
    pub suppressions_active: i64,
    /// Commercial unsubscribes (marketing scope) in the window.
    pub unsubscribes: i64,
    /// `None` when there is no deliverability event in the window (no webhook
    /// ingestion on this instance, or no traffic).
    pub bounce_rate_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub channels: Vec<String>,
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    pub rate_limit_per_min: i32,
    pub sent_in_window: i64,
    pub failed_in_window: i64,
}

pub async fn digest(state: &Arc<AppState>, window: Duration) -> Result<Digest> {
    let since = Utc::now() - window;
    let pool = &state.pool;

    // Queue
    let mut queue = QueueState::default();
    let depth: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, COUNT(*) FROM jobs WHERE status IN ('pending','retry','processing') GROUP BY status",
    )
    .fetch_all(pool)
    .await?;
    for (status, count) in depth {
        match status.as_str() {
            "pending" => queue.pending = count,
            "retry" => queue.retry = count,
            "processing" => queue.processing = count,
            _ => {}
        }
    }
    let ages: Vec<(String, Option<f64>)> = sqlx::query_as(
        r#"
        SELECT band, EXTRACT(EPOCH FROM now() - MIN(scheduled_at))::float8
        FROM (
            SELECT scheduled_at,
                   CASE WHEN priority < 50 THEN 'urgent' WHEN priority < 80 THEN 'normal' ELSE 'bulk' END AS band
            FROM jobs WHERE status IN ('pending','retry') AND scheduled_at <= now()
        ) waiting GROUP BY band
        "#,
    )
    .fetch_all(pool)
    .await?;
    queue.oldest_waiting_seconds = ages
        .into_iter()
        .map(|(band, secs)| BandAge {
            band,
            seconds: secs.unwrap_or(0.0).max(0.0) as i64,
        })
        .collect();

    // Outcomes in window
    let outcome_rows = sqlx::query(
        r#"
        SELECT channel, COALESCE(provider, 'unknown') AS provider,
               COUNT(*) FILTER (WHERE status = 'sent') AS sent,
               COUNT(*) FILTER (WHERE status = 'failed') AS failed
        FROM jobs
        WHERE status IN ('sent','failed') AND COALESCE(sent_at, created_at) >= $1
        GROUP BY channel, COALESCE(provider, 'unknown')
        ORDER BY channel, provider
        "#,
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    let outcomes: Vec<OutcomeRow> = outcome_rows
        .iter()
        .map(|r| OutcomeRow {
            channel: r.get("channel"),
            provider: r.get("provider"),
            sent: r.get("sent"),
            failed: r.get("failed"),
        })
        .collect();

    // Failures grouped by normalised reason
    let failure_rows = sqlx::query(
        r#"
        SELECT channel,
               regexp_replace(COALESCE(error, 'unknown'), '[0-9a-f]{8}-[0-9a-f-]{27}|[0-9]{3,}', '#', 'g') AS reason,
               COUNT(*) AS count,
               (array_agg(id ORDER BY created_at DESC))[1] AS sample_job_id,
               MAX(created_at) AS last_seen
        FROM jobs
        WHERE status = 'failed' AND created_at >= $1
        GROUP BY channel, reason
        ORDER BY count DESC
        LIMIT 10
        "#,
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    let failures: Vec<FailureGroup> = failure_rows
        .iter()
        .map(|r| FailureGroup {
            channel: r.get("channel"),
            reason: truncate(r.get::<String, _>("reason"), 160),
            count: r.get("count"),
            sample_job_id: r.get("sample_job_id"),
            last_seen: r.get("last_seen"),
        })
        .collect();

    // Retry backlog
    let retry_rows = sqlx::query(
        "SELECT channel, attempts, COUNT(*) AS count, MIN(next_retry_at) AS next_retry_at
         FROM jobs WHERE status = 'retry' GROUP BY channel, attempts ORDER BY channel, attempts",
    )
    .fetch_all(pool)
    .await?;
    let retries_waiting: Vec<RetryRow> = retry_rows
        .iter()
        .map(|r| RetryRow {
            channel: r.get("channel"),
            attempts: r.get("attempts"),
            count: r.get("count"),
            next_retry_at: r.get("next_retry_at"),
        })
        .collect();

    // Latency scheduled_at → sent_at
    let latency_rows = sqlx::query(
        r#"
        SELECT channel, COUNT(*) AS sent,
               percentile_cont(0.5) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM sent_at - scheduled_at)) AS p50,
               percentile_cont(0.95) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM sent_at - scheduled_at)) AS p95
        FROM jobs
        WHERE status = 'sent' AND sent_at >= $1 AND sent_at >= scheduled_at
        GROUP BY channel ORDER BY channel
        "#,
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    let latency: Vec<LatencyRow> = latency_rows
        .iter()
        .map(|r| LatencyRow {
            channel: r.get("channel"),
            sent: r.get("sent"),
            p50_seconds: r.get::<Option<f64>, _>("p50").unwrap_or(0.0),
            p95_seconds: r.get::<Option<f64>, _>("p95").unwrap_or(0.0),
        })
        .collect();

    // Deliverability (email provider events + suppressions)
    let mut deliverability = Deliverability::default();
    let events: Vec<(String, i64)> = sqlx::query_as(
        "SELECT event_type, COUNT(*) FROM provider_events WHERE received_at >= $1 GROUP BY event_type",
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    for (event_type, count) in events {
        match event_type.as_str() {
            "email.delivered" => deliverability.delivered = count,
            "email.bounced" => deliverability.bounced = count,
            "email.complained" => deliverability.complained = count,
            _ => {}
        }
    }
    deliverability.suppressions_added =
        sqlx::query_scalar("SELECT COUNT(*) FROM email_suppressions WHERE created_at >= $1")
            .bind(since)
            .fetch_one(pool)
            .await?;
    deliverability.suppressions_active =
        sqlx::query_scalar("SELECT COUNT(*) FROM email_suppressions WHERE released_at IS NULL")
            .fetch_one(pool)
            .await?;
    deliverability.unsubscribes = sqlx::query_scalar(
        "SELECT COUNT(*) FROM email_suppressions WHERE reason = 'unsubscribe' AND created_at >= $1",
    )
    .bind(since)
    .fetch_one(pool)
    .await?;
    let observed = deliverability.delivered + deliverability.bounced;
    deliverability.bounce_rate_percent = (observed > 0)
        .then(|| (deliverability.bounced as f64 / observed as f64 * 100.0 * 100.0).round() / 100.0);

    // Projects
    let project_rows = sqlx::query(
        r#"
        SELECT p.id, p.name, p.channels, p.from_email, p.from_name, p.rate_limit_per_min,
               COALESCE(j.sent, 0) AS sent, COALESCE(j.failed, 0) AS failed
        FROM projects p
        LEFT JOIN (
            SELECT project_id,
                   COUNT(*) FILTER (WHERE status = 'sent') AS sent,
                   COUNT(*) FILTER (WHERE status = 'failed') AS failed
            FROM jobs WHERE COALESCE(sent_at, created_at) >= $1 GROUP BY project_id
        ) j ON j.project_id = p.id
        ORDER BY p.id
        "#,
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    let projects: Vec<ProjectRow> = project_rows
        .iter()
        .map(|r| ProjectRow {
            id: r.get("id"),
            name: r.get("name"),
            channels: r.get::<Vec<String>, _>("channels"),
            from_email: r.get("from_email"),
            from_name: r.get("from_name"),
            rate_limit_per_min: r.get("rate_limit_per_min"),
            sent_in_window: r.get("sent"),
            failed_in_window: r.get("failed"),
        })
        .collect();

    let workflows_active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_runs WHERE status IN ('running', 'paused')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let instance = InstanceInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("NOTIFYD_GIT_COMMIT"),
        built_at_epoch: env!("NOTIFYD_BUILD_EPOCH").parse().unwrap_or(0),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        email_provider: state
            .config
            .connectors
            .email
            .as_ref()
            .map(|c| c.provider.clone()),
        sms_provider: state
            .config
            .connectors
            .sms
            .as_ref()
            .map(|c| c.provider.clone()),
        whatsapp_provider: state
            .config
            .connectors
            .whatsapp
            .as_ref()
            .map(|c| c.provider.clone()),
        paused_lanes: state.pacer.paused_channels(),
        email_fallback_provider: state
            .config
            .connectors
            .email_fallback
            .as_ref()
            .map(|c| c.provider.clone()),
        email_primary_resting_seconds: state.email_breaker.open_for().map(|d| d.as_secs()),
        email_failovers_since_boot: state.email_breaker.trips(),
        public_url: crate::unsubscribe::public_url(),
    };

    let findings = compute_findings(
        &instance,
        &queue,
        &outcomes,
        &failures,
        &deliverability,
        &projects,
        window,
    );

    Ok(Digest {
        generated_at: Utc::now(),
        window: window_label(window),
        instance,
        findings,
        queue,
        outcomes,
        failures,
        retries_waiting,
        latency,
        deliverability,
        projects,
        workflows_active,
    })
}

/// The part an agent reads first. Rules are deliberately simple and
/// explained in `action`, so the reader can disagree with them.
fn compute_findings(
    instance: &InstanceInfo,
    queue: &QueueState,
    outcomes: &[OutcomeRow],
    failures: &[FailureGroup],
    deliverability: &Deliverability,
    projects: &[ProjectRow],
    window: Duration,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    if instance.email_provider.is_none() {
        findings.push(Finding {
            severity: "critical",
            message: "No email provider configured: every email job fails.".into(),
            action: "Set EMAIL_PROVIDER (and its credentials) on the instance.".into(),
        });
    }
    if instance.email_provider.as_deref() == Some("log") {
        findings.push(Finding {
            severity: "critical",
            message: "Email provider is `log`: nothing is actually sent.".into(),
            action: "Development setting. Switch EMAIL_PROVIDER to a real provider before serving customers.".into(),
        });
    }
    if let Some(secs) = instance.email_primary_resting_seconds {
        findings.push(Finding {
            severity: "warning",
            message: format!(
                "Primary email provider `{}` is resting for {secs}s after refusing messages; `{}` is delivering.",
                instance.email_provider.as_deref().unwrap_or("?"),
                instance.email_fallback_provider.as_deref().unwrap_or("?")
            ),
            action: "Nothing lost. Check the primary provider's status page; if it repeats, lower EMAIL_RATE_PER_SEC or move the primary role to the other provider.".into(),
        });
    }
    if instance.email_provider.is_some() && instance.public_url.is_none() {
        findings.push(Finding {
            severity: "warning",
            message: "PUBLIC_URL is not set: bulk email leaves without List-Unsubscribe headers.".into(),
            action: "Set PUBLIC_URL to this instance's public base URL; Gmail and Yahoo require one-click unsubscribe on bulk senders.".into(),
        });
    }
    if instance.email_fallback_provider.is_none() && instance.email_provider.is_some() {
        findings.push(Finding {
            severity: "info",
            message: "No fallback email provider: a provider incident delays emails until it ends.".into(),
            action: "Set EMAIL_FALLBACK_PROVIDER (smtp, cloudflare, resend…) with its credentials; the sender domain must be verified there too.".into(),
        });
    }
    for lane in &instance.paused_lanes {
        findings.push(Finding {
            severity: "warning",
            message: format!("Lane `{lane}` is paused after a provider 429."),
            action: "Transient. If it repeats, lower the lane's rate (EMAIL_RATE_PER_SEC…) or ask the provider for a higher limit.".into(),
        });
    }
    for age in &queue.oldest_waiting_seconds {
        let threshold = if age.band == "bulk" { 3600 } else { 300 };
        if age.seconds > threshold {
            findings.push(Finding {
                severity: if age.band == "urgent" { "critical" } else { "warning" },
                message: format!(
                    "Oldest `{}` job has been waiting {} minutes.",
                    age.band,
                    age.seconds / 60
                ),
                action: "Check the worker is running and the lane is not paused; look at retries_waiting for the cause.".into(),
            });
        }
    }
    if queue.processing > 0 && queue.pending == 0 && queue.retry == 0 && outcomes.is_empty() {
        findings.push(Finding {
            severity: "warning",
            message: format!(
                "{} job(s) in processing and nothing sent in the window.",
                queue.processing
            ),
            action: "A worker may have died mid-batch; the reaper re-queues after 10 minutes."
                .into(),
        });
    }

    let total_sent: i64 = outcomes.iter().map(|o| o.sent).sum();
    let total_failed: i64 = outcomes.iter().map(|o| o.failed).sum();
    if total_failed > 0 {
        let rate = total_failed as f64 / (total_sent + total_failed).max(1) as f64 * 100.0;
        let severity = if rate >= 10.0 {
            "critical"
        } else if rate >= 2.0 {
            "warning"
        } else {
            "info"
        };
        let top = failures
            .first()
            .map(|f| format!(" Top reason: {} ({}×).", f.reason, f.count))
            .unwrap_or_default();
        findings.push(Finding {
            severity,
            message: format!(
                "{total_failed} job(s) failed in the last {} ({rate:.1} % of terminal jobs).{top}",
                window_label(window)
            ),
            action: "Inspect with list_jobs(status=failed); permanent errors (bad address, unverified sender) need a fix on the caller side, then retry_job.".into(),
        });
    }
    if let Some(rate) = deliverability.bounce_rate_percent {
        if rate >= 5.0 {
            findings.push(Finding {
                severity: "critical",
                message: format!("Bounce rate {rate:.1} % over the window ({} bounced / {} delivered).", deliverability.bounced, deliverability.delivered),
                action: "Above 5 % providers throttle or suspend the sender. Clean the recipient list; suppressions are applied automatically.".into(),
            });
        } else if rate >= 2.0 {
            findings.push(Finding {
                severity: "warning",
                message: format!("Bounce rate {rate:.1} % over the window."),
                action: "Watch list quality; 2 % is the usual alert threshold.".into(),
            });
        }
    }
    if deliverability.complained > 0 {
        findings.push(Finding {
            severity: "warning",
            message: format!("{} spam complaint(s) in the window.", deliverability.complained),
            action: "Complainants are suppressed automatically. Review the content and frequency of the campaign involved.".into(),
        });
    }
    for project in projects {
        if project.channels.iter().any(|c| c == "email") && project.from_email.is_none() {
            findings.push(Finding {
                severity: "warning",
                message: format!(
                    "Project `{}` has no from_email: its emails use the instance default sender.",
                    project.id
                ),
                action: "Set from_email/from_name with update_project so the brand and reply address are right.".into(),
            });
        }
    }
    if findings.is_empty() {
        findings.push(Finding {
            severity: "info",
            message: format!(
                "All quiet: {total_sent} sent, 0 failed in the last {}.",
                window_label(window)
            ),
            action: "Nothing to do.".into(),
        });
    }
    let rank = |s: &str| match s {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    };
    findings.sort_by_key(|f| rank(f.severity));
    findings
}

fn window_label(window: Duration) -> String {
    if window.num_hours() < 24 {
        format!("{}h", window.num_hours())
    } else {
        format!("{}d", window.num_days())
    }
}

/// Markdown rendering of the digest: what a human wants to read, what an
/// agent can paste into a report.
pub fn render_markdown(d: &Digest) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# notifyd digest — last {} (generated {})\n\n",
        d.window,
        d.generated_at.format("%Y-%m-%d %H:%M UTC")
    ));
    out.push_str(&format!(
        "Instance: commit `{}`, up {}, email `{}`, sms `{}`, whatsapp `{}`{}\n\n",
        d.instance.commit,
        human_duration(d.instance.uptime_seconds),
        d.instance.email_provider.as_deref().unwrap_or("none"),
        d.instance.sms_provider.as_deref().unwrap_or("none"),
        d.instance.whatsapp_provider.as_deref().unwrap_or("none"),
        match (
            &d.instance.email_fallback_provider,
            d.instance.email_primary_resting_seconds
        ) {
            (Some(fb), Some(secs)) =>
                format!(", email fallback `{fb}` ACTIVE ({secs}s left on primary rest)"),
            (Some(fb), None) => format!(", email fallback `{fb}`"),
            (None, _) => String::new(),
        } + &if d.instance.paused_lanes.is_empty() {
            String::new()
        } else {
            format!(", paused lanes: {}", d.instance.paused_lanes.join(", "))
        }
    ));

    out.push_str("## Findings\n\n");
    for f in &d.findings {
        out.push_str(&format!(
            "- **{}** — {} _{}_\n",
            f.severity, f.message, f.action
        ));
    }

    out.push_str("\n## Queue\n\n");
    out.push_str(&format!(
        "pending {}, retry {}, processing {}",
        d.queue.pending, d.queue.retry, d.queue.processing
    ));
    if !d.queue.oldest_waiting_seconds.is_empty() {
        let ages: Vec<String> = d
            .queue
            .oldest_waiting_seconds
            .iter()
            .map(|a| format!("{} {}", a.band, human_duration(a.seconds.max(0) as u64)))
            .collect();
        out.push_str(&format!(" — oldest waiting: {}", ages.join(", ")));
    }
    out.push_str(
        "\n\n## Outcomes\n\n| channel | provider | sent | failed |\n|---|---|---:|---:|\n",
    );
    for o in &d.outcomes {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            o.channel, o.provider, o.sent, o.failed
        ));
    }
    if d.outcomes.is_empty() {
        out.push_str("| — | — | 0 | 0 |\n");
    }
    if !d.failures.is_empty() {
        out.push_str("\n## Failures (top reasons)\n\n| channel | count | reason | sample job |\n|---|---:|---|---|\n");
        for f in &d.failures {
            out.push_str(&format!(
                "| {} | {} | {} | `{}` |\n",
                f.channel,
                f.count,
                f.reason.replace('|', "\\|"),
                f.sample_job_id
            ));
        }
    }
    if !d.retries_waiting.is_empty() {
        out.push_str("\n## Retries waiting\n\n| channel | attempts | count | next retry |\n|---|---:|---:|---|\n");
        for r in &d.retries_waiting {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                r.channel,
                r.attempts,
                r.count,
                r.next_retry_at
                    .map(|t| t.format("%H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "—".into())
            ));
        }
    }
    if !d.latency.is_empty() {
        out.push_str("\n## Latency (scheduled → accepted by provider)\n\n| channel | sent | p50 | p95 |\n|---|---:|---:|---:|\n");
        for l in &d.latency {
            out.push_str(&format!(
                "| {} | {} | {:.1}s | {:.1}s |\n",
                l.channel, l.sent, l.p50_seconds, l.p95_seconds
            ));
        }
    }
    out.push_str(&format!(
        "\n## Deliverability\n\ndelivered {}, bounced {}, complained {}, unsubscribed {}, bounce rate {}, suppressions added {} (active {})\n",
        d.deliverability.delivered,
        d.deliverability.bounced,
        d.deliverability.complained,
        d.deliverability.unsubscribes,
        d.deliverability
            .bounce_rate_percent
            .map(|r| format!("{r:.2} %"))
            .unwrap_or_else(|| "n/a".into()),
        d.deliverability.suppressions_added,
        d.deliverability.suppressions_active
    ));
    out.push_str("\n## Projects\n\n| project | channels | sender | sent | failed |\n|---|---|---|---:|---:|\n");
    for p in &d.projects {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            p.id,
            p.channels.join(", "),
            match (&p.from_email, &p.from_name) {
                (Some(e), Some(n)) => format!("{n} <{e}>"),
                (Some(e), None) => e.clone(),
                _ => "instance default".into(),
            },
            p.sent_in_window,
            p.failed_in_window
        ));
    }
    out.push_str(&format!("\nActive workflow runs: {}\n", d.workflows_active));
    out
}

fn human_duration(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h{:02}", s / 3600, (s % 3600) / 60),
        s => format!("{}d{}h", s / 86_400, (s % 86_400) / 3600),
    }
}

fn truncate(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        text
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    }
}

// ─── Jobs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct JobFilter {
    pub project_id: Option<String>,
    pub status: Option<String>,
    pub channel: Option<String>,
    pub recipient: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

pub async fn list_jobs(state: &Arc<AppState>, filter: &JobFilter) -> Result<Vec<Value>> {
    let limit = filter.limit.unwrap_or(50).clamp(1, 500);
    let since = filter
        .since
        .unwrap_or_else(|| Utc::now() - Duration::days(7));
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, channel, status, recipient, priority, attempts, max_attempts,
               provider, provider_message_id, scheduled_at, sent_at, next_retry_at, created_at, error,
               payload->>'subject' AS subject
        FROM jobs
        WHERE created_at >= $1
          AND ($2::text IS NULL OR project_id = $2)
          AND ($3::text IS NULL OR status = $3)
          AND ($4::text IS NULL OR channel = $4)
          AND ($5::text IS NULL OR lower(recipient) = lower($5))
        ORDER BY created_at DESC
        LIMIT $6
        "#,
    )
    .bind(since)
    .bind(&filter.project_id)
    .bind(&filter.status)
    .bind(&filter.channel)
    .bind(&filter.recipient)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "project_id": r.get::<String, _>("project_id"),
                "channel": r.get::<String, _>("channel"),
                "status": r.get::<String, _>("status"),
                "recipient": crate::pii::mask_recipient(r.get::<String, _>("channel").as_str(), &r.get::<String, _>("recipient")),
                "subject": r.get::<Option<String>, _>("subject"),
                "priority": r.get::<i16, _>("priority"),
                "attempts": r.get::<i32, _>("attempts"),
                "max_attempts": r.get::<i32, _>("max_attempts"),
                "provider": r.get::<Option<String>, _>("provider"),
                "provider_message_id": r.get::<Option<String>, _>("provider_message_id"),
                "scheduled_at": r.get::<DateTime<Utc>, _>("scheduled_at"),
                "sent_at": r.get::<Option<DateTime<Utc>>, _>("sent_at"),
                "next_retry_at": r.get::<Option<DateTime<Utc>>, _>("next_retry_at"),
                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
                "error": r.get::<Option<String>, _>("error"),
            })
        })
        .collect())
}

/// One job with its provider events, admin scope (no project restriction).
pub async fn get_job(state: &Arc<AppState>, job_id: Uuid) -> Result<Value> {
    let row = sqlx::query(&format!(
        "SELECT {}, delivered_at, bounced_at FROM jobs WHERE id = $1",
        crate::db::JOB_COLUMNS
    ))
    .bind(job_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| anyhow!("job {job_id} not found"))?;
    let events = sqlx::query(
        "SELECT provider, event_type, received_at FROM provider_events WHERE job_id = $1 ORDER BY received_at",
    )
    .bind(job_id)
    .fetch_all(&state.pool)
    .await?;
    let channel: String = row.get("channel");
    Ok(json!({
        "id": row.get::<Uuid, _>("id"),
        "project_id": row.get::<String, _>("project_id"),
        "channel": channel,
        "status": row.get::<String, _>("status"),
        "recipient": crate::pii::mask_recipient(&channel, &row.get::<String, _>("recipient")),
        "subject": row.get::<Value, _>("payload").get("subject").cloned(),
        "priority": row.get::<i16, _>("priority"),
        "attempts": row.get::<i32, _>("attempts"),
        "max_attempts": row.get::<i32, _>("max_attempts"),
        "provider": row.get::<Option<String>, _>("provider"),
        "provider_message_id": row.get::<Option<String>, _>("provider_message_id"),
        "scheduled_at": row.get::<DateTime<Utc>, _>("scheduled_at"),
        "claimed_at": row.get::<Option<DateTime<Utc>>, _>("claimed_at"),
        "sent_at": row.get::<Option<DateTime<Utc>>, _>("sent_at"),
        "delivered_at": row.get::<Option<DateTime<Utc>>, _>("delivered_at"),
        "bounced_at": row.get::<Option<DateTime<Utc>>, _>("bounced_at"),
        "next_retry_at": row.get::<Option<DateTime<Utc>>, _>("next_retry_at"),
        "error": row.get::<Option<String>, _>("error"),
        "provider_events": events.iter().map(|e| json!({
            "provider": e.get::<String, _>("provider"),
            "type": e.get::<String, _>("event_type"),
            "received_at": e.get::<DateTime<Utc>, _>("received_at"),
        })).collect::<Vec<_>>(),
    }))
}

/// Enqueue a test message with high priority and a `category=test` tag, so
/// an agent can prove a channel end to end without touching the callers.
pub async fn enqueue_test(
    state: &Arc<AppState>,
    project_id: &str,
    channel: &str,
    to: &str,
    subject: Option<&str>,
    body: &str,
) -> Result<Value> {
    if crate::connectors::Channel::from_str(channel).is_none() {
        return Err(anyhow!("unknown channel {channel}"));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1)")
        .bind(project_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(anyhow!("project {project_id} not found"));
    }
    let subject = subject.unwrap_or("notifyd test");
    let payload = json!({
        "subject": subject,
        "body": body,
        "body_html": format!("<p>{}</p>", body),
        "icon": "bell",
        "tags": [{"name": "category", "value": "test"}],
    });
    let subscriber_id = if channel == "in_app" || channel == "push" {
        Some(to)
    } else {
        None
    };
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO jobs (project_id, channel, subscriber_id, recipient, payload, scheduled_at, priority, max_attempts)
        VALUES ($1, $2, $3, $4, $5, now(), $6, $7)
        RETURNING id
        "#,
    )
    .bind(project_id)
    .bind(channel)
    .bind(subscriber_id)
    .bind(to)
    .bind(&payload)
    .bind(crate::api::send::PRIORITY_HIGH)
    .bind(state.config.worker.max_attempts)
    .fetch_one(&state.pool)
    .await?;
    Ok(json!({ "id": id, "project_id": project_id, "channel": channel, "status": "pending" }))
}

pub async fn list_projects(state: &Arc<AppState>) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        "SELECT id, name, channels, from_email, from_name, rate_limit_per_min, settings, created_at FROM projects ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<String, _>("name"),
                "channels": r.get::<Vec<String>, _>("channels"),
                "from_email": r.get::<Option<String>, _>("from_email"),
                "from_name": r.get::<Option<String>, _>("from_name"),
                "rate_limit_per_min": r.get::<i32, _>("rate_limit_per_min"),
                "send_window": r.get::<Option<Value>, _>("settings").and_then(|s| s.get("send_window").cloned()),
                "created_at": r.get::<Option<DateTime<Utc>>, _>("created_at"),
            })
        })
        .collect())
}

/// Put a `failed` or `cancelled` job back in the queue with a fresh attempt
/// budget. The error history is kept on the row. `project_id` scopes the
/// operation when the caller holds a project key; admins pass `None`.
pub async fn retry_job(
    state: &Arc<AppState>,
    job_id: Uuid,
    project_id: Option<&str>,
) -> Result<Value> {
    let row = sqlx::query(
        r#"
        UPDATE jobs
        SET status = 'retry', attempts = 0, next_retry_at = now(), claimed_at = NULL,
            error = COALESCE(error, '') || ' [retried manually]'
        WHERE id = $1
          AND status IN ('failed', 'cancelled')
          AND ($2::text IS NULL OR project_id = $2)
        RETURNING id, project_id, channel, status
        "#,
    )
    .bind(job_id)
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        Some(r) => Ok(json!({
            "id": r.get::<Uuid, _>("id"),
            "project_id": r.get::<String, _>("project_id"),
            "channel": r.get::<String, _>("channel"),
            "status": r.get::<String, _>("status"),
        })),
        None => Err(anyhow!(
            "job {job_id} not found, not yours, or not in a retryable state (failed, cancelled)"
        )),
    }
}

pub async fn cancel_job(
    state: &Arc<AppState>,
    job_id: Uuid,
    project_id: Option<&str>,
) -> Result<Value> {
    let row = sqlx::query(
        r#"
        UPDATE jobs SET status = 'cancelled'
        WHERE id = $1 AND status IN ('pending', 'retry')
          AND ($2::text IS NULL OR project_id = $2)
        RETURNING id, project_id, channel, status
        "#,
    )
    .bind(job_id)
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        Some(r) => Ok(json!({
            "id": r.get::<Uuid, _>("id"),
            "project_id": r.get::<String, _>("project_id"),
            "channel": r.get::<String, _>("channel"),
            "status": r.get::<String, _>("status"),
        })),
        None => Err(anyhow!(
            "job {job_id} not found, not yours, or already processing/terminal"
        )),
    }
}

// ─── Suppressions ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuppressionScope {
    /// Nothing is sent to the address (bounce, complaint, manual block).
    All,
    /// Bulk email stops, transactional email still goes (commercial unsubscribe).
    Marketing,
}

impl SuppressionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Marketing => "marketing",
        }
    }
    pub fn parse(raw: Option<&str>) -> Result<Self> {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("all") => Ok(Self::All),
            Some("marketing") => Ok(Self::Marketing),
            Some(other) => Err(anyhow!("scope must be all or marketing (got {other})")),
        }
    }
}

pub async fn add_suppression(
    state: &Arc<AppState>,
    project_id: &str,
    email: &str,
    detail: Option<&str>,
    actor: &str,
    scope: SuppressionScope,
) -> Result<Value> {
    let email = email.trim().to_lowercase();
    if !email.contains('@') {
        return Err(anyhow!("invalid email address"));
    }
    let reason = match scope {
        SuppressionScope::All => "manual",
        SuppressionScope::Marketing => "unsubscribe",
    };
    // One active row per address: a wider scope wins over a narrower one,
    // a repeated unsubscribe is a no-op.
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO email_suppressions (project_id, email, reason, detail, scope)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (project_id, lower(email)) WHERE released_at IS NULL
        DO UPDATE SET
            detail = EXCLUDED.detail,
            scope = CASE WHEN email_suppressions.scope = 'all' THEN 'all' ELSE EXCLUDED.scope END,
            reason = CASE WHEN email_suppressions.scope = 'all' THEN email_suppressions.reason ELSE EXCLUDED.reason END
        RETURNING id
        "#,
    )
    .bind(project_id)
    .bind(&email)
    .bind(reason)
    .bind(detail.map(|d| format!("{d} (by {actor})")))
    .bind(scope.as_str())
    .fetch_one(&state.pool)
    .await?;
    Ok(json!({
        "id": id,
        "project_id": project_id,
        "email": crate::pii::mask_email(&email),
        "reason": reason,
        "scope": scope.as_str(),
    }))
}

pub async fn list_suppressions(
    state: &Arc<AppState>,
    project_id: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>> {
    let rows = sqlx::query(
        r#"
        SELECT id, project_id, email, reason, detail, scope, created_at
        FROM email_suppressions
        WHERE released_at IS NULL AND ($1::text IS NULL OR project_id = $1)
        ORDER BY created_at DESC LIMIT $2
        "#,
    )
    .bind(project_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "project_id": r.get::<String, _>("project_id"),
                "email": crate::pii::mask_email(&r.get::<String, _>("email")),
                "reason": r.get::<String, _>("reason"),
                "scope": r.get::<String, _>("scope"),
                "detail": r.get::<Option<String>, _>("detail"),
                "created_at": r.get::<DateTime<Utc>, _>("created_at"),
            })
        })
        .collect())
}

pub async fn release_suppression(
    state: &Arc<AppState>,
    id: Uuid,
    project_id: Option<&str>,
    actor: &str,
) -> Result<Value> {
    let released = sqlx::query(
        r#"
        UPDATE email_suppressions SET released_at = now(), released_by = $3
        WHERE id = $1 AND released_at IS NULL AND ($2::text IS NULL OR project_id = $2)
        RETURNING id
        "#,
    )
    .bind(id)
    .bind(project_id)
    .bind(actor)
    .fetch_optional(&state.pool)
    .await?;
    match released {
        Some(_) => Ok(json!({ "id": id, "released": true })),
        None => Err(anyhow!("suppression {id} not found or already released")),
    }
}

// ─── Projects ───────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub channels: Option<Vec<String>>,
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    pub rate_limit_per_min: Option<i32>,
    /// Daily send window for bulk email, `{start, end, tz?, days?,
    /// applies_to?}`; `null` removes it. Stored in `settings.send_window`.
    pub send_window: Option<Value>,
}

/// Update a project's settings without touching its keys.
pub async fn update_project(
    state: &Arc<AppState>,
    project_id: &str,
    patch: &ProjectPatch,
) -> Result<Value> {
    if let Some(email) = &patch.from_email {
        if !email.trim().is_empty() && !email.contains('@') {
            return Err(anyhow!("from_email must be an email address"));
        }
    }
    if let Some(channels) = &patch.channels {
        for c in channels {
            if crate::connectors::Channel::from_str(c).is_none() {
                return Err(anyhow!("unknown channel {c}"));
            }
        }
    }
    let from_email = patch
        .from_email
        .as_ref()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    let clear_from_email = matches!(patch.from_email.as_deref().map(str::trim), Some(""));
    // send_window: validate before storing; null clears; absent leaves as is.
    let (set_window, window_value) = match &patch.send_window {
        None => (false, Value::Null),
        Some(Value::Null) => (true, Value::Null),
        Some(v) => {
            crate::send_window::SendWindow::parse(v)?;
            (true, v.clone())
        }
    };
    let row = sqlx::query(
        r#"
        UPDATE projects
        SET name = COALESCE($2, name),
            channels = COALESCE($3, channels),
            from_email = CASE WHEN $7 THEN NULL ELSE COALESCE($4, from_email) END,
            from_name = COALESCE($5, from_name),
            rate_limit_per_min = COALESCE($6, rate_limit_per_min),
            settings = CASE
                WHEN NOT $8 THEN settings
                WHEN $9::jsonb IS NULL OR $9::jsonb = 'null'::jsonb THEN COALESCE(settings, '{}'::jsonb) - 'send_window'
                ELSE COALESCE(settings, '{}'::jsonb) || jsonb_build_object('send_window', $9::jsonb)
            END,
            updated_at = now()
        WHERE id = $1
        RETURNING id, name, channels, from_email, from_name, rate_limit_per_min, settings
        "#,
    )
    .bind(project_id)
    .bind(&patch.name)
    .bind(&patch.channels)
    .bind(&from_email)
    .bind(&patch.from_name)
    .bind(patch.rate_limit_per_min)
    .bind(clear_from_email)
    .bind(set_window)
    .bind(window_value)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        Some(r) => Ok(json!({
            "id": r.get::<String, _>("id"),
            "name": r.get::<String, _>("name"),
            "channels": r.get::<Vec<String>, _>("channels"),
            "from_email": r.get::<Option<String>, _>("from_email"),
            "from_name": r.get::<Option<String>, _>("from_name"),
            "rate_limit_per_min": r.get::<i32, _>("rate_limit_per_min"),
            "send_window": r.get::<Option<Value>, _>("settings").and_then(|s| s.get("send_window").cloned()),
        })),
        None => Err(anyhow!("project {project_id} not found")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_bounded() {
        assert_eq!(parse_window(None).unwrap(), Duration::hours(24));
        assert_eq!(parse_window(Some("7d")).unwrap(), Duration::days(7));
        assert!(parse_window(Some("3y")).is_err());
    }

    #[test]
    fn findings_rank_critical_first_and_explain_actions() {
        let instance = InstanceInfo {
            version: "0",
            commit: "abc",
            built_at_epoch: 0,
            uptime_seconds: 10,
            email_provider: Some("log".into()),
            sms_provider: None,
            whatsapp_provider: None,
            paused_lanes: vec!["email".into()],
            email_fallback_provider: None,
            email_primary_resting_seconds: None,
            email_failovers_since_boot: 0,
            public_url: None,
        };
        let queue = QueueState::default();
        let outcomes = vec![OutcomeRow {
            channel: "email".into(),
            provider: "log".into(),
            sent: 90,
            failed: 10,
        }];
        let failures = vec![FailureGroup {
            channel: "email".into(),
            reason: "HTTP 422".into(),
            count: 10,
            sample_job_id: Uuid::nil(),
            last_seen: Utc::now(),
        }];
        let deliverability = Deliverability {
            bounce_rate_percent: Some(6.0),
            ..Default::default()
        };
        let projects = vec![ProjectRow {
            id: "p".into(),
            name: "P".into(),
            channels: vec!["email".into()],
            from_email: None,
            from_name: None,
            rate_limit_per_min: 600,
            sent_in_window: 0,
            failed_in_window: 0,
        }];
        let findings = compute_findings(
            &instance,
            &queue,
            &outcomes,
            &failures,
            &deliverability,
            &projects,
            Duration::hours(24),
        );
        assert_eq!(findings[0].severity, "critical");
        assert!(findings.iter().all(|f| !f.action.is_empty()));
        assert!(findings
            .iter()
            .any(|f| f.message.contains("10 job(s) failed")));
        assert!(findings.iter().any(|f| f.message.contains("no from_email")));
        let last = findings.last().unwrap();
        assert_ne!(last.severity, "critical");
    }

    #[test]
    fn quiet_instance_says_so() {
        let instance = InstanceInfo {
            version: "0",
            commit: "abc",
            built_at_epoch: 0,
            uptime_seconds: 10,
            email_provider: Some("resend".into()),
            sms_provider: None,
            whatsapp_provider: None,
            paused_lanes: vec![],
            email_fallback_provider: Some("smtp".into()),
            email_primary_resting_seconds: None,
            email_failovers_since_boot: 0,
            public_url: Some("https://n.example.com".into()),
        };
        let findings = compute_findings(
            &instance,
            &QueueState::default(),
            &[OutcomeRow {
                channel: "email".into(),
                provider: "resend".into(),
                sent: 12,
                failed: 0,
            }],
            &[],
            &Deliverability::default(),
            &[],
            Duration::hours(24),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.starts_with("All quiet: 12 sent"));
    }

    #[test]
    fn human_durations() {
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(600), "10m");
        assert_eq!(human_duration(3_720), "1h02");
        assert_eq!(human_duration(90_000), "1d1h");
    }
}
