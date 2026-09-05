//! Prometheus metrics. Counters are process-wide; gauges for queue depth
//! are refreshed from the database when `/v1/metrics/prometheus` is read,
//! so the scrape always reflects the queue and not a cached snapshot.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec, Encoder,
    HistogramVec, IntCounterVec, IntGaugeVec, TextEncoder,
};

pub static JOBS_OUTCOME: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "notifyd_jobs_outcome_total",
        "Job dispatch outcomes by channel, provider and outcome (sent, retry, failed, rate_limited, skipped)",
        &["channel", "provider", "outcome"]
    )
    .expect("metric registration")
});

pub static PROVIDER_ERRORS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "notifyd_provider_errors_total",
        "Provider errors by channel, provider and kind (rate_limited, transient, permanent, suppressed)",
        &["channel", "provider", "kind"]
    )
    .expect("metric registration")
});

pub static LANE_PAUSES: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "notifyd_lane_pauses_total",
        "Times a channel lane was paused after a provider 429",
        &["channel"]
    )
    .expect("metric registration")
});

pub static FAILOVERS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "notifyd_email_failovers_total",
        "Emails re-routed from the primary to the fallback provider, by outcome (sent, failed)",
        &["from", "to", "outcome"]
    )
    .expect("metric registration")
});

pub static SEND_LATENCY: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "notifyd_send_latency_seconds",
        "Provider call latency by channel and provider",
        &["channel", "provider"],
        vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 20.0]
    )
    .expect("metric registration")
});

pub static QUEUE_DEPTH: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "notifyd_jobs_queue_depth",
        "Jobs by status (pending, retry, processing) at scrape time",
        &["status"]
    )
    .expect("metric registration")
});

pub static OLDEST_PENDING_AGE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "notifyd_oldest_pending_age_seconds",
        "Age of the oldest job still waiting, by priority band (urgent <50, normal 50-79, bulk >=80)",
        &["band"]
    )
    .expect("metric registration")
});

pub fn record_outcome(channel: &str, provider: &str, outcome: &str) {
    JOBS_OUTCOME
        .with_label_values(&[channel, provider, outcome])
        .inc();
}

pub fn record_provider_error(channel: &str, provider: &str, kind: &str) {
    PROVIDER_ERRORS
        .with_label_values(&[channel, provider, kind])
        .inc();
}

pub fn record_failover(from: &str, to: &str, outcome: &str) {
    FAILOVERS.with_label_values(&[from, to, outcome]).inc();
}

pub fn record_lane_pause(channel: &str) {
    LANE_PAUSES.with_label_values(&[channel]).inc();
}

pub fn observe_latency(channel: &str, provider: &str, seconds: f64) {
    SEND_LATENCY
        .with_label_values(&[channel, provider])
        .observe(seconds);
}

/// Refresh queue gauges from the database, then render the registry.
pub async fn render(pool: &sqlx::PgPool) -> String {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, COUNT(*) FROM jobs WHERE status IN ('pending','retry','processing') GROUP BY status",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for status in ["pending", "retry", "processing"] {
        let count = rows
            .iter()
            .find(|(s, _)| s == status)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        QUEUE_DEPTH.with_label_values(&[status]).set(count);
    }

    let ages: Vec<(String, Option<f64>)> = sqlx::query_as(
        r#"
        SELECT band, EXTRACT(EPOCH FROM now() - MIN(scheduled_at))::float8
        FROM (
            SELECT scheduled_at,
                   CASE WHEN priority < 50 THEN 'urgent'
                        WHEN priority < 80 THEN 'normal'
                        ELSE 'bulk' END AS band
            FROM jobs
            WHERE status IN ('pending','retry') AND scheduled_at <= now()
        ) waiting
        GROUP BY band
        "#,
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for band in ["urgent", "normal", "bulk"] {
        let age = ages
            .iter()
            .find(|(b, _)| b == band)
            .and_then(|(_, a)| *a)
            .unwrap_or(0.0);
        OLDEST_PENDING_AGE
            .with_label_values(&[band])
            .set(age.max(0.0) as i64);
    }

    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    if encoder.encode(&prometheus::gather(), &mut buffer).is_err() {
        return String::new();
    }
    String::from_utf8(buffer).unwrap_or_default()
}
