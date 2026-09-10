//! The digest, delivered where the operator already is: a Telegram chat, a
//! Slack channel or webhook, a Discord webhook. Same findings as
//! `GET /v1/admin/digest`, as a short text a phone can show.
//!
//! - `DIGEST_NOTIFY=telegram:<chat_id>` | `slack:<webhook or channel id>` |
//!   `discord:<webhook url>` — the destination.
//! - `DIGEST_NOTIFY_EVERY=1d` — how often (`30m`, `6h`, `1d`; default `1d`).
//! - `DIGEST_NOTIFY_LEVEL=warning` — send only when a finding reaches this
//!   severity (`critical`, `warning`, `info`); `always` sends every time.
//!
//! `POST /v1/admin/digest/notify {"to"?, "window"?}` and `notifyd digest
//! --to telegram:123` send one right away through the same code.

use crate::connectors::{chat::ChatConnector, Channel, Connector, Delivery, SendRequest};
use crate::ops::{self, Digest};
use crate::AppState;
use anyhow::{anyhow, Result};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

/// `telegram:123` → (Telegram, "123").
pub fn parse_target(target: &str) -> Result<(Channel, String)> {
    let (channel, recipient) = target
        .trim()
        .split_once(':')
        .ok_or_else(|| anyhow!("DIGEST_NOTIFY must be telegram:<chat id>, slack:<webhook or channel> or discord:<webhook>"))?;
    let recipient = recipient.trim();
    if recipient.is_empty() {
        return Err(anyhow!(
            "DIGEST_NOTIFY: empty destination after '{channel}:'"
        ));
    }
    let channel = match channel.trim().to_ascii_lowercase().as_str() {
        "telegram" => Channel::Telegram,
        "slack" => Channel::Slack,
        "discord" => Channel::Discord,
        other => {
            return Err(anyhow!(
                "DIGEST_NOTIFY: unknown channel '{other}' (telegram, slack, discord)"
            ))
        }
    };
    Ok((channel, recipient.to_string()))
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 3,
        "warning" => 2,
        "info" => 1,
        _ => 0,
    }
}

/// Whether the digest is worth a message at `level` (`always` = yes).
pub fn should_send(digest: &Digest, level: &str) -> bool {
    if level.eq_ignore_ascii_case("always") {
        return true;
    }
    let threshold = severity_rank(&level.to_ascii_lowercase()).max(1);
    digest
        .findings
        .iter()
        .any(|f| severity_rank(f.severity) >= threshold)
}

/// Short text: what needs attention first, then the numbers that frame it.
pub fn render_chat(d: &Digest) -> (String, String) {
    let worst = d
        .findings
        .iter()
        .map(|f| severity_rank(f.severity))
        .max()
        .unwrap_or(0);
    let headline = match worst {
        3 => format!(
            "notifyd: {} critical",
            d.findings
                .iter()
                .filter(|f| f.severity == "critical")
                .count()
        ),
        2 => format!(
            "notifyd: {} warning(s)",
            d.findings
                .iter()
                .filter(|f| f.severity == "warning")
                .count()
        ),
        _ => "notifyd: all quiet".to_string(),
    };
    let mut body = String::new();
    body.push_str(&format!(
        "last {} · {}\n",
        d.window,
        d.generated_at.format("%Y-%m-%d %H:%M UTC")
    ));
    if d.findings.is_empty() {
        body.push_str("No findings.\n");
    } else {
        for f in d.findings.iter().take(6) {
            let mark = match f.severity {
                "critical" => "!!",
                "warning" => "!",
                _ => "·",
            };
            body.push_str(&format!("{mark} {}\n   → {}\n", f.message, f.action));
        }
        if d.findings.len() > 6 {
            body.push_str(&format!("… {} more\n", d.findings.len() - 6));
        }
    }
    let sent: i64 = d.outcomes.iter().map(|o| o.sent).sum();
    let failed: i64 = d.outcomes.iter().map(|o| o.failed).sum();
    body.push_str(&format!(
        "\nqueue pending {} · retry {} · processing {}\nsent {} · failed {}",
        d.queue.pending, d.queue.retry, d.queue.processing, sent, failed
    ));
    if d.deliverability.delivered + d.deliverability.bounced > 0 {
        body.push_str(&format!(
            "\ndelivered {} · bounced {} · complained {}",
            d.deliverability.delivered, d.deliverability.bounced, d.deliverability.complained
        ));
    }
    if !d.instance.paused_lanes.is_empty() {
        body.push_str(&format!(
            "\npaused lanes: {}",
            d.instance.paused_lanes.join(", ")
        ));
    }
    (headline, body)
}

/// Compute the digest over `window` and send it to `target` now.
pub async fn send(
    state: &Arc<AppState>,
    target: &str,
    window: chrono::Duration,
) -> Result<Delivery> {
    let (channel, recipient) = parse_target(target)?;
    let digest = ops::digest(state, window).await?;
    let (headline, body) = render_chat(&digest);
    let connector = ChatConnector::new(channel, state.config.connectors.chat.clone());
    let req = SendRequest {
        recipient,
        subject: Some(headline),
        body,
        body_html: None,
        from_email: None,
        from_name: None,
        metadata: json!({}),
    };
    connector
        .send(&req)
        .await
        .map_err(|e| anyhow!("{}: {}", e.provider, e.message))
}

/// `30m`, `6h`, `1d` → seconds.
pub fn parse_every(raw: &str) -> Result<u64> {
    let raw = raw.trim();
    let (num, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow!("DIGEST_NOTIFY_EVERY: use 30m, 6h or 1d"))?;
    let secs = match unit {
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return Err(anyhow!("DIGEST_NOTIFY_EVERY: use 30m, 6h or 1d")),
    };
    if secs < 300 {
        return Err(anyhow!("DIGEST_NOTIFY_EVERY: at least 5m"));
    }
    Ok(secs)
}

/// Background loop driven by `DIGEST_NOTIFY*`; returns at once when unset.
pub fn spawn_scheduler(state: Arc<AppState>) -> Result<()> {
    let Some(target) = std::env::var("DIGEST_NOTIFY")
        .ok()
        .filter(|t| !t.trim().is_empty())
    else {
        return Ok(());
    };
    parse_target(&target)?;
    let every =
        parse_every(&std::env::var("DIGEST_NOTIFY_EVERY").unwrap_or_else(|_| "1d".to_string()))?;
    let level = std::env::var("DIGEST_NOTIFY_LEVEL").unwrap_or_else(|_| "warning".to_string());
    let window = chrono::Duration::seconds(every as i64);
    info!(
        "Digest notifications: {} every {}s when a finding is {} or worse",
        target.split_once(':').map(|(c, _)| c).unwrap_or("?"),
        every,
        level
    );
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(every));
        tick.tick().await; // the first tick fires immediately; skip it
        loop {
            tick.tick().await;
            match ops::digest(&state, window).await {
                Ok(digest) if !should_send(&digest, &level) => {
                    info!(
                        "Digest notification skipped: nothing at level {} or worse",
                        level
                    );
                }
                Ok(_) => match send(&state, &target, window).await {
                    Ok(d) => info!("Digest sent via {}", d.provider),
                    Err(e) => warn!("Digest notification failed: {}", e),
                },
                Err(e) => warn!("Digest computation failed: {}", e),
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_intervals() {
        assert_eq!(
            parse_target("telegram:123").unwrap(),
            (Channel::Telegram, "123".to_string())
        );
        assert_eq!(
            parse_target(" slack:https://hooks.slack.com/services/T/B/x ")
                .unwrap()
                .1,
            "https://hooks.slack.com/services/T/B/x"
        );
        assert!(parse_target("sms:+33612345678").is_err());
        assert!(parse_target("telegram:").is_err());
        assert!(parse_target("nope").is_err());
        assert_eq!(parse_every("30m").unwrap(), 1800);
        assert_eq!(parse_every("1d").unwrap(), 86_400);
        assert!(parse_every("1m").is_err());
        assert!(parse_every("soon").is_err());
    }

    #[test]
    fn level_gate() {
        let mut d = ops::Digest::empty_for_tests("1d");
        assert!(!should_send(&d, "warning"));
        assert!(should_send(&d, "always"));
        d.findings.push(ops::Finding {
            severity: "info",
            message: "fyi".into(),
            action: "nothing".into(),
        });
        assert!(!should_send(&d, "warning"));
        assert!(should_send(&d, "info"));
        d.findings.push(ops::Finding {
            severity: "warning",
            message: "bounce rate 5.3 %".into(),
            action: "clean the list".into(),
        });
        assert!(should_send(&d, "warning"));
        assert!(!should_send(&d, "critical"));
        let (headline, body) = render_chat(&d);
        assert_eq!(headline, "notifyd: 1 warning(s)");
        assert!(body.contains("! bounce rate 5.3 %\n   → clean the list"));
        assert!(body.contains("queue pending 0"));
    }
}
