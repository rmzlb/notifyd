//! `notifyd <subcommand>`: the operator's job from a terminal, against a
//! running instance, with the same admin API the MCP tools use. No arguments
//! = start the server, as before.
//!
//! ```text
//! notifyd digest [--window 1d] [--json]
//! notifyd jobs [--status failed] [--project P] [--channel email] [--topic t] [--recipient a@b] [--since 24h] [--limit 50] [--json]
//! notifyd job <id> [--json]
//! notifyd retry <id>
//! notifyd cancel <id>
//! notifyd send-test --project P --channel email --to a@b [--subject S] [--body B]
//! notifyd version | help
//! ```
//!
//! Target and credentials: `--url` / `NOTIFYD_URL` (default
//! `http://localhost:3400`), `--key` / `NOTIFYD_ADMIN_API_KEY` (falls back to
//! `ADMIN_API_KEY`, so it just works on the server host).

use serde_json::Value;
use std::collections::HashMap;

const HELP: &str = "notifyd — self-hosted notification server

  notifyd                      start the server (reads the environment)
  notifyd digest               what needs attention, with the action for each finding
  notifyd jobs                 recent jobs; filters: --status --project --channel --topic --recipient --since --limit
  notifyd job <id>             one job: provider, attempts, delivery events
  notifyd retry <id>           re-queue a failed job
  notifyd cancel <id>          cancel a pending or scheduled job
  notifyd send-test            prove a channel: --project P --channel email|sms|whatsapp|in_app|push --to X [--subject S] [--body B]
  notifyd version

Options: --url URL (NOTIFYD_URL), --key KEY (NOTIFYD_ADMIN_API_KEY or ADMIN_API_KEY), --json, --window 1d (digest), --since 24h (jobs)";

/// Runs the subcommand when there is one. `Ok(false)` = no subcommand, start
/// the server. Errors are printed by the caller with a non-zero exit code.
pub async fn run(args: &[String]) -> anyhow::Result<bool> {
    let Some(command) = args.first() else {
        return Ok(false);
    };
    let (flags, positional) = parse(&args[1..]);
    let json = flags.contains_key("json");
    match command.as_str() {
        "help" | "--help" | "-h" => {
            println!("{HELP}");
            Ok(true)
        }
        "version" | "--version" | "-V" => {
            println!("notifyd {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        "digest" => {
            let client = Client::from(&flags)?;
            let window = flags
                .get("window")
                .cloned()
                .flatten()
                .unwrap_or_else(|| "1d".to_string());
            if json {
                let v = client
                    .get(&format!("/v1/admin/digest?window={window}"))
                    .await?;
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                let text = client
                    .get_text(&format!("/v1/admin/digest?window={window}&format=markdown"))
                    .await?;
                println!("{text}");
            }
            Ok(true)
        }
        "jobs" => {
            let client = Client::from(&flags)?;
            let mut query: Vec<String> = Vec::new();
            for (flag, param) in [
                ("status", "status"),
                ("project", "project_id"),
                ("channel", "channel"),
                ("topic", "topic"),
                ("recipient", "recipient"),
                ("limit", "limit"),
            ] {
                if let Some(Some(v)) = flags.get(flag) {
                    query.push(format!("{param}={}", urlencode(v)));
                }
            }
            if let Some(Some(since)) = flags.get("since") {
                query.push(format!("since={}", urlencode(&since_to_rfc3339(since)?)));
            }
            let v = client
                .get(&format!("/v1/admin/jobs?{}", query.join("&")))
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print_jobs(&v);
            }
            Ok(true)
        }
        "job" => {
            let id = positional
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: notifyd job <id>"))?;
            let client = Client::from(&flags)?;
            let v = client.get(&format!("/v1/admin/jobs/{id}")).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print_job(&v);
            }
            Ok(true)
        }
        "retry" | "cancel" => {
            let id = positional
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: notifyd {command} <id>"))?;
            let client = Client::from(&flags)?;
            let v = client
                .post(&format!("/v1/admin/jobs/{id}/{command}"), Value::Null)
                .await?;
            let status = v
                .pointer("/job/status")
                .and_then(Value::as_str)
                .unwrap_or("?");
            println!("{command}: job {id} is now {status}");
            Ok(true)
        }
        "send-test" => {
            let client = Client::from(&flags)?;
            let need = |name: &str| -> anyhow::Result<String> {
                flags
                    .get(name)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| anyhow::anyhow!("send-test needs --{name}"))
            };
            let body = serde_json::json!({
                "project_id": need("project")?,
                "channel": need("channel")?,
                "to": need("to")?,
                "subject": flags.get("subject").cloned().flatten(),
                "body": flags.get("body").cloned().flatten(),
            });
            let v = client.post("/v1/admin/send-test", body).await?;
            println!(
                "queued job {} on {} — follow it with: notifyd job {}",
                v.get("id").and_then(Value::as_str).unwrap_or("?"),
                v.get("channel").and_then(Value::as_str).unwrap_or("?"),
                v.get("id").and_then(Value::as_str).unwrap_or("<id>")
            );
            Ok(true)
        }
        other => anyhow::bail!("unknown command '{other}'\n\n{HELP}"),
    }
}

/// Flags that never take a value.
const BOOL_FLAGS: &[&str] = &["json", "help"];

/// `--flag value` / `--flag=value` / bare `--flag`; the rest is positional.
fn parse(args: &[String]) -> (HashMap<String, Option<String>>, Vec<String>) {
    let mut flags: HashMap<String, Option<String>> = HashMap::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if let Some((k, v)) = name.split_once('=') {
                flags.insert(k.to_string(), Some(v.to_string()));
            } else if BOOL_FLAGS.contains(&name) {
                flags.insert(name.to_string(), None);
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.insert(name.to_string(), Some(args[i + 1].clone()));
                i += 1;
            } else {
                flags.insert(name.to_string(), None);
            }
        } else {
            positional.push(a.clone());
        }
        i += 1;
    }
    (flags, positional)
}

/// `24h`, `7d`, `30m` or an RFC 3339 timestamp → RFC 3339.
fn since_to_rfc3339(value: &str) -> anyhow::Result<String> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(t.to_rfc3339());
    }
    let (num, unit) = value.split_at(value.len().saturating_sub(1));
    let n: i64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("--since: use 30m, 24h, 7d or an RFC 3339 timestamp"))?;
    let secs = match unit {
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => anyhow::bail!("--since: use 30m, 24h, 7d or an RFC 3339 timestamp"),
    };
    Ok((chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339())
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

struct Client {
    url: String,
    key: String,
    http: reqwest::Client,
}

impl Client {
    fn from(flags: &HashMap<String, Option<String>>) -> anyhow::Result<Self> {
        let url = flags
            .get("url")
            .cloned()
            .flatten()
            .or_else(|| std::env::var("NOTIFYD_URL").ok())
            .unwrap_or_else(|| "http://localhost:3400".to_string())
            .trim_end_matches('/')
            .to_string();
        let key = flags
            .get("key")
            .cloned()
            .flatten()
            .or_else(|| std::env::var("NOTIFYD_ADMIN_API_KEY").ok())
            .or_else(|| std::env::var("ADMIN_API_KEY").ok())
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no admin key: pass --key or set NOTIFYD_ADMIN_API_KEY (or ADMIN_API_KEY)"
                )
            })?;
        Ok(Self {
            url,
            key,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
        })
    }

    async fn get_text(&self, path: &str) -> anyhow::Result<String> {
        let res = self
            .http
            .get(format!("{}{}", self.url, path))
            .header("X-Api-Key", &self.key)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}: {e}", self.url))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("{} {path}: {}", status.as_u16(), short(&text));
        }
        Ok(text)
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        Ok(serde_json::from_str(&self.get_text(path).await?)?)
    }

    async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let mut req = self
            .http
            .post(format!("{}{}", self.url, path))
            .header("X-Api-Key", &self.key);
        if !body.is_null() {
            req = req.json(&body);
        }
        let res = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}: {e}", self.url))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("{} {path}: {}", status.as_u16(), short(&text));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

fn short(text: &str) -> String {
    let t: String = text.chars().take(300).collect();
    serde_json::from_str::<Value>(&t)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
        .unwrap_or(t)
}

fn cell(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "-".to_string(),
        Some(other) => other.to_string(),
    }
}

fn print_jobs(v: &Value) {
    let rows = v
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if rows.is_empty() {
        println!("no jobs match");
        return;
    }
    println!(
        "{:<36}  {:<19}  {:<8}  {:<10}  {:<9}  {:<26}  {}",
        "id", "created", "channel", "status", "provider", "recipient", "subject / error"
    );
    for r in &rows {
        let created = cell(r, "created_at");
        let tail = match r.get("error").and_then(Value::as_str) {
            Some(e) if !e.is_empty() => format!("! {}", e.chars().take(60).collect::<String>()),
            _ => cell(r, "subject").chars().take(60).collect(),
        };
        println!(
            "{:<36}  {:<19}  {:<8}  {:<10}  {:<9}  {:<26}  {}",
            cell(r, "id"),
            created.chars().take(19).collect::<String>(),
            cell(r, "channel"),
            cell(r, "status"),
            cell(r, "provider"),
            cell(r, "recipient").chars().take(26).collect::<String>(),
            tail
        );
    }
    println!("{} job(s)", rows.len());
}

fn print_job(v: &Value) {
    for key in [
        "id",
        "project_id",
        "channel",
        "status",
        "recipient",
        "topic",
        "priority",
        "attempts",
        "max_attempts",
        "provider",
        "provider_message_id",
        "scheduled_at",
        "sent_at",
        "delivered_at",
        "opened_at",
        "clicked_at",
        "bounced_at",
        "error",
    ] {
        if let Some(val) = v.get(key) {
            if !val.is_null() {
                println!("{key:<20} {}", cell(v, key));
            }
        }
    }
    if let Some(events) = v.get("provider_events").and_then(Value::as_array) {
        if !events.is_empty() {
            println!("events");
            for e in events {
                println!(
                    "  {:<19} {:<8} {}",
                    cell(e, "occurred_at").chars().take(19).collect::<String>(),
                    cell(e, "provider"),
                    cell(e, "type")
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_and_positionals() {
        let (flags, pos) = parse(&[
            "--status".into(),
            "failed".into(),
            "--json".into(),
            "abc".into(),
            "--limit=5".into(),
        ]);
        assert_eq!(flags.get("status"), Some(&Some("failed".to_string())));
        assert_eq!(flags.get("json"), Some(&None));
        assert_eq!(flags.get("limit"), Some(&Some("5".to_string())));
        assert_eq!(pos, vec!["abc".to_string()]);
    }

    #[test]
    fn since_accepts_durations_and_timestamps() {
        assert!(since_to_rfc3339("24h").unwrap().starts_with("20"));
        assert!(since_to_rfc3339("7d").is_ok());
        assert!(since_to_rfc3339("30m").is_ok());
        assert_eq!(
            since_to_rfc3339("2026-09-10T00:00:00+00:00").unwrap(),
            "2026-09-10T00:00:00+00:00"
        );
        assert!(since_to_rfc3339("yesterday").is_err());
    }

    #[test]
    fn urlencode_keeps_safe_characters() {
        assert_eq!(urlencode("a@b.c"), "a%40b.c");
        assert_eq!(urlencode("release-notes_v2.1~x"), "release-notes_v2.1~x");
    }

    #[tokio::test]
    async fn no_arguments_means_server() {
        assert!(!run(&[]).await.unwrap());
    }
}
