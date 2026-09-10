//! Topics: the subscriber-facing name of a stream of messages ("tips",
//! "release-notes", "billing"). A job carries at most one topic, taken from
//! the request or from its template. Preferences reuse the
//! `subscriber_preferences` scope column (`workflow_id`): a row whose scope is
//! a topic applies to every job with that topic, whatever produced it.
//!
//! Decision order, most specific first:
//!   1. (channel, topic)        2. ("*", topic)
//!   3. (channel, workflow)     4. (channel, "*")      5. ("*", "*")
//! No matching row: allowed.

use serde_json::Value;

/// Topic ids are short, lowercase, URL-safe: they end up in preference rows,
/// unsubscribe links and digest tables.
pub fn normalize(topic: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = topic else { return Ok(None) };
    let t = raw.trim().to_ascii_lowercase();
    if t.is_empty() {
        return Ok(None);
    }
    if t == "*" {
        return Err("topic must not be '*'".to_string());
    }
    if t.len() > 64
        || !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(
            "topic must be 1–64 characters: letters, digits, '-', '_', '.' or ':'".to_string(),
        );
    }
    Ok(Some(t))
}

/// One preference row as stored: (channel, scope, enabled) where scope is a
/// topic id, a workflow id or "*".
pub type PreferenceRow = (String, String, bool);

/// Whether a message may go out on `channel`, given the subscriber's rows.
pub fn allowed(
    rows: &[PreferenceRow],
    channel: &str,
    topic: Option<&str>,
    workflow: Option<&str>,
) -> bool {
    let find = |c: &str, scope: &str| {
        rows.iter()
            .find(|(rc, rs, _)| rc == c && rs == scope)
            .map(|(_, _, enabled)| *enabled)
    };
    if let Some(t) = topic {
        if let Some(v) = find(channel, t).or_else(|| find("*", t)) {
            return v;
        }
    }
    if let Some(w) = workflow {
        if let Some(v) = find(channel, w) {
            return v;
        }
    }
    find(channel, "*")
        .or_else(|| find("*", "*"))
        .unwrap_or(true)
}

/// Human explanation for a skipped channel, for API responses and logs.
pub fn skip_reason(channel: &str, topic: Option<&str>) -> String {
    match topic {
        Some(t) => format!("subscriber opted out of topic '{t}' on {channel}"),
        None => format!("subscriber opted out of {channel}"),
    }
}

/// Topic from the request, else from the template when one is named.
pub async fn resolve(
    pool: &sqlx::PgPool,
    project_id: &str,
    requested: Option<&str>,
    template: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(t) = normalize(requested)? {
        return Ok(Some(t));
    }
    let Some(tmpl) = template else {
        return Ok(None);
    };
    let topic: Option<Option<String>> = sqlx::query_scalar(
        "SELECT topic FROM templates WHERE project_id = $1 AND id = $2 AND topic IS NOT NULL LIMIT 1",
    )
    .bind(project_id)
    .bind(tmpl)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(topic.flatten())
}

/// Preference rows of several subscribers in one query, keyed by subscriber.
pub async fn load_rows(
    pool: &sqlx::PgPool,
    project_id: &str,
    subscribers: &[String],
) -> Result<std::collections::HashMap<String, Vec<PreferenceRow>>, sqlx::Error> {
    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
        "SELECT subscriber_id, channel, workflow_id, enabled FROM subscriber_preferences
         WHERE project_id = $1 AND subscriber_id = ANY($2)",
    )
    .bind(project_id)
    .bind(subscribers)
    .fetch_all(pool)
    .await?;
    let mut map: std::collections::HashMap<String, Vec<PreferenceRow>> = Default::default();
    for (sub, channel, scope, enabled) in rows {
        map.entry(sub).or_default().push((channel, scope, enabled));
    }
    Ok(map)
}

/// `scope` of a preference row from an API body: `topic` wins over
/// `workflow_id`, both default to "*".
pub fn scope_from(topic: Option<&str>, workflow_id: Option<&str>) -> Result<String, String> {
    if let Some(t) = normalize(topic)? {
        return Ok(t);
    }
    Ok(workflow_id
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .unwrap_or("*")
        .to_string())
}

/// JSON view of a stored row, with the scope under both names.
pub fn row_json(channel: &str, scope: &str, enabled: bool) -> Value {
    serde_json::json!({
        "channel": channel,
        "workflow_id": scope,
        "topic": if scope == "*" { Value::Null } else { Value::String(scope.to_string()) },
        "enabled": enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[(&str, &str, bool)]) -> Vec<PreferenceRow> {
        items
            .iter()
            .map(|(c, s, e)| (c.to_string(), s.to_string(), *e))
            .collect()
    }

    #[test]
    fn normalizes_and_validates_topics() {
        assert_eq!(normalize(Some(" Tips ")).unwrap(), Some("tips".into()));
        assert_eq!(normalize(Some("")).unwrap(), None);
        assert_eq!(normalize(None).unwrap(), None);
        assert!(normalize(Some("*")).is_err());
        assert!(normalize(Some("has space")).is_err());
        assert!(normalize(Some(&"x".repeat(65))).is_err());
        assert_eq!(
            normalize(Some("release-notes.v2:eu")).unwrap(),
            Some("release-notes.v2:eu".into())
        );
    }

    #[test]
    fn topic_row_beats_workflow_channel_and_global() {
        let r = rows(&[("email", "*", false), ("email", "tips", true)]);
        assert!(
            allowed(&r, "email", Some("tips"), None),
            "topic opt-in wins over channel opt-out"
        );
        let r = rows(&[("email", "tips", false)]);
        assert!(!allowed(&r, "email", Some("tips"), Some("welcome")));
        assert!(
            allowed(&r, "email", Some("billing"), None),
            "another topic is untouched"
        );
        assert!(
            allowed(&r, "sms", Some("tips"), None),
            "another channel is untouched"
        );
    }

    #[test]
    fn star_channel_topic_row_covers_every_channel() {
        let r = rows(&[("*", "tips", false)]);
        assert!(!allowed(&r, "email", Some("tips"), None));
        assert!(!allowed(&r, "push", Some("tips"), None));
        assert!(allowed(&r, "push", None, None));
    }

    #[test]
    fn legacy_precedence_is_unchanged_without_topic() {
        let r = rows(&[("email", "welcome", false), ("email", "*", true)]);
        assert!(!allowed(&r, "email", None, Some("welcome")));
        assert!(allowed(&r, "email", None, Some("other")));
        let r = rows(&[("*", "*", false)]);
        assert!(!allowed(&r, "sms", None, None));
        assert!(allowed(&[], "sms", Some("tips"), Some("wf")));
    }

    #[test]
    fn scope_prefers_topic_then_workflow_then_star() {
        assert_eq!(scope_from(Some("Tips"), Some("wf")).unwrap(), "tips");
        assert_eq!(scope_from(None, Some("wf")).unwrap(), "wf");
        assert_eq!(scope_from(None, None).unwrap(), "*");
        assert_eq!(scope_from(Some(""), Some(" ")).unwrap(), "*");
        assert!(scope_from(Some("bad topic"), None).is_err());
    }
}
