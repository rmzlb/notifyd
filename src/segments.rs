//! Segments: send a batch to "every subscriber matching this filter" instead
//! of listing ids. The filter is a small, closed vocabulary compiled to SQL
//! with bound parameters; nothing from the request is ever spliced into the
//! query text except column names chosen from a whitelist.
//!
//! ```json
//! { "locale": ["fr", "be"], "timezone": "Europe/Paris", "has_email": true,
//!   "created_after": "2026-01-01T00:00:00Z",
//!   "data": { "plan": "pro", "country": ["FR", "BE"] },
//!   "where": [ { "field": "data.seats", "op": "gte", "value": 5 } ] }
//! ```
//! `data` is sugar for `eq` / `in` on `data.<key>`; `where` is the general
//! form. Fields: `locale`, `timezone`, `email`, `phone`, `first_name`,
//! `last_name`, `created_at`, `data.<path>`. Operators: `eq`, `neq`, `in`,
//! `not_in`, `exists`, `contains`, `gt`, `gte`, `lt`, `lte`.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
pub struct Segment {
    pub locale: Option<Value>,
    pub timezone: Option<Value>,
    pub has_email: Option<bool>,
    pub has_phone: Option<bool>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub data: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub r#where: Vec<Condition>,
}

#[derive(Debug, Deserialize)]
pub struct Condition {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: Value,
}

/// A bound parameter of the compiled filter.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    Text(String),
    TextArray(Vec<String>),
    Timestamp(DateTime<Utc>),
}

/// `WHERE` fragment (without the keyword) and its binds. `$1` is reserved for
/// the project id by every caller; binds here start at `$2`.
pub fn compile(segment: &Segment) -> Result<(String, Vec<Bind>), String> {
    let mut clauses: Vec<String> = vec!["project_id = $1".to_string()];
    let mut binds: Vec<Bind> = Vec::new();
    let mut next = |binds: &mut Vec<Bind>, b: Bind| -> String {
        binds.push(b);
        format!("${}", binds.len() + 1)
    };

    if let Some(v) = &segment.locale {
        clauses.push(text_match("locale", v, &mut binds, &mut next)?);
    }
    if let Some(v) = &segment.timezone {
        clauses.push(text_match("timezone", v, &mut binds, &mut next)?);
    }
    if let Some(true) = segment.has_email {
        clauses.push("email IS NOT NULL AND email <> ''".to_string());
    }
    if let Some(false) = segment.has_email {
        clauses.push("(email IS NULL OR email = '')".to_string());
    }
    if let Some(true) = segment.has_phone {
        clauses.push("phone IS NOT NULL AND phone <> ''".to_string());
    }
    if let Some(false) = segment.has_phone {
        clauses.push("(phone IS NULL OR phone = '')".to_string());
    }
    if let Some(t) = segment.created_after {
        let p = next(&mut binds, Bind::Timestamp(t));
        clauses.push(format!("created_at >= {p}"));
    }
    if let Some(t) = segment.created_before {
        let p = next(&mut binds, Bind::Timestamp(t));
        clauses.push(format!("created_at < {p}"));
    }
    if let Some(data) = &segment.data {
        for (key, v) in data {
            let expr = data_expr(key)?;
            clauses.push(text_match(&expr, v, &mut binds, &mut next)?);
        }
    }
    for c in &segment.r#where {
        clauses.push(condition(c, &mut binds, &mut next)?);
    }
    Ok((clauses.join(" AND "), binds))
}

/// `field = $n` for a scalar, `field = ANY($n)` for an array.
fn text_match(
    expr: &str,
    value: &Value,
    binds: &mut Vec<Bind>,
    next: &mut impl FnMut(&mut Vec<Bind>, Bind) -> String,
) -> Result<String, String> {
    match value {
        Value::Array(items) => {
            let list = items
                .iter()
                .map(scalar_text)
                .collect::<Result<Vec<_>, _>>()?;
            if list.is_empty() {
                return Err(format!("{expr}: empty list"));
            }
            let p = next(binds, Bind::TextArray(list));
            Ok(format!("{expr} = ANY({p}::text[])"))
        }
        other => {
            let p = next(binds, Bind::Text(scalar_text(other)?));
            Ok(format!("{expr} = {p}"))
        }
    }
}

/// JSON scalars compare as their text form, the way `->>` yields them.
fn scalar_text(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err("segment values must be strings, numbers or booleans".to_string()),
    }
}

/// `data.plan` → `data->>'plan'`; `data.a.b` → `data#>>'{a,b}'`. Keys are
/// restricted to identifier characters so they can be inlined safely.
fn data_expr(path: &str) -> Result<String, String> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty()
        || parts.iter().any(|p| {
            p.is_empty()
                || p.len() > 64
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
    {
        return Err(format!(
            "data key '{path}': letters, digits, '_' and '-' only, dot-separated"
        ));
    }
    Ok(if parts.len() == 1 {
        format!("data->>'{}'", parts[0])
    } else {
        format!("data#>>'{{{}}}'", parts.join(","))
    })
}

/// Column expression for a `where` field.
fn field_expr(field: &str) -> Result<(String, bool), String> {
    Ok(match field {
        "locale" | "timezone" | "email" | "phone" | "first_name" | "last_name" => {
            (field.to_string(), false)
        }
        "created_at" => ("created_at".to_string(), true),
        f if f.starts_with("data.") => (data_expr(&f[5..])?, false),
        other => {
            return Err(format!(
                "unknown field '{other}' (locale, timezone, email, phone, first_name, last_name, created_at, data.<key>)"
            ))
        }
    })
}

fn condition(
    c: &Condition,
    binds: &mut Vec<Bind>,
    next: &mut impl FnMut(&mut Vec<Bind>, Bind) -> String,
) -> Result<String, String> {
    let (expr, is_timestamp) = field_expr(&c.field)?;
    let numeric = |expr: &str| {
        // Non-numeric text never matches a numeric comparison instead of erroring.
        format!("CASE WHEN {expr} ~ '^-?[0-9]+(\\.[0-9]+)?$' THEN ({expr})::numeric END")
    };
    match c.op.as_str() {
        "eq" => text_match(&expr, &c.value, binds, next),
        "neq" => {
            let p = next(binds, Bind::Text(scalar_text(&c.value)?));
            Ok(format!("({expr} IS NULL OR {expr} <> {p})"))
        }
        "in" => match &c.value {
            Value::Array(_) => text_match(&expr, &c.value, binds, next),
            _ => Err(format!("{}: 'in' needs a list", c.field)),
        },
        "not_in" => match &c.value {
            Value::Array(items) => {
                let list = items
                    .iter()
                    .map(scalar_text)
                    .collect::<Result<Vec<_>, _>>()?;
                let p = next(binds, Bind::TextArray(list));
                Ok(format!(
                    "({expr} IS NULL OR NOT ({expr} = ANY({p}::text[])))"
                ))
            }
            _ => Err(format!("{}: 'not_in' needs a list", c.field)),
        },
        "exists" => Ok(match c.value {
            Value::Bool(false) => format!("{expr} IS NULL"),
            _ => format!("{expr} IS NOT NULL"),
        }),
        "contains" => {
            let needle = scalar_text(&c.value)?
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let p = next(binds, Bind::Text(format!("%{needle}%")));
            Ok(format!("{expr} ILIKE {p}"))
        }
        op @ ("gt" | "gte" | "lt" | "lte") => {
            let sym = match op {
                "gt" => ">",
                "gte" => ">=",
                "lt" => "<",
                _ => "<=",
            };
            if is_timestamp {
                let t: DateTime<Utc> = serde_json::from_value(c.value.clone())
                    .map_err(|_| format!("{}: {op} needs an RFC 3339 timestamp", c.field))?;
                let p = next(binds, Bind::Timestamp(t));
                Ok(format!("{expr} {sym} {p}"))
            } else {
                let n = c
                    .value
                    .as_f64()
                    .ok_or_else(|| format!("{}: {op} needs a number", c.field))?;
                let p = next(binds, Bind::Text(n.to_string()));
                Ok(format!("{} {sym} ({p})::numeric", numeric(&expr)))
            }
        }
        other => Err(format!(
            "unknown operator '{other}' (eq, neq, in, not_in, exists, contains, gt, gte, lt, lte)"
        )),
    }
}

fn apply_binds<'q, O>(
    mut q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    project_id: &'q str,
    binds: &'q [Bind],
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    q = q.bind(project_id);
    for b in binds {
        q = match b {
            Bind::Text(s) => q.bind(s),
            Bind::TextArray(v) => q.bind(v),
            Bind::Timestamp(t) => q.bind(t),
        };
    }
    q
}

/// Matching subscribers: (id, timezone), oldest first.
pub async fn resolve(
    pool: &sqlx::PgPool,
    project_id: &str,
    segment: &Segment,
) -> Result<Vec<(String, Option<String>)>, String> {
    let (where_sql, binds) = compile(segment)?;
    let sql =
        format!("SELECT id, timezone FROM subscribers WHERE {where_sql} ORDER BY created_at, id");
    apply_binds(
        sqlx::query_as::<_, (String, Option<String>)>(&sql),
        project_id,
        &binds,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("segment query failed: {}", e);
        "segment query failed".to_string()
    })
}

/// How many subscribers match, plus a few ids to eyeball.
pub async fn preview(
    pool: &sqlx::PgPool,
    project_id: &str,
    segment: &Segment,
) -> Result<(i64, Vec<String>), String> {
    let (where_sql, binds) = compile(segment)?;
    let count_sql = format!("SELECT COUNT(*) FROM subscribers WHERE {where_sql}");
    let (count,): (i64,) = apply_binds(sqlx::query_as(&count_sql), project_id, &binds)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            tracing::error!("segment count failed: {}", e);
            "segment query failed".to_string()
        })?;
    let sample_sql =
        format!("SELECT id FROM subscribers WHERE {where_sql} ORDER BY created_at, id LIMIT 10");
    let sample: Vec<(String,)> = apply_binds(sqlx::query_as(&sample_sql), project_id, &binds)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            tracing::error!("segment sample failed: {}", e);
            "segment query failed".to_string()
        })?;
    Ok((count, sample.into_iter().map(|(id,)| id).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seg(v: Value) -> Segment {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn sugar_fields_compile_to_bound_parameters() {
        let (sql, binds) = compile(&seg(json!({
            "locale": ["fr", "be"], "timezone": "Europe/Paris", "has_email": true,
            "created_after": "2026-01-01T00:00:00Z",
            "data": {"plan": "pro", "seats": 5, "beta": true}
        })))
        .unwrap();
        assert_eq!(
            sql,
            "project_id = $1 AND locale = ANY($2::text[]) AND timezone = $3 AND email IS NOT NULL AND email <> '' AND created_at >= $4 AND data->>'beta' = $5 AND data->>'plan' = $6 AND data->>'seats' = $7"
        );
        assert_eq!(binds.len(), 6);
        assert_eq!(binds[0], Bind::TextArray(vec!["fr".into(), "be".into()]));
        assert_eq!(binds[3], Bind::Text("true".into()));
        assert_eq!(binds[5], Bind::Text("5".into()));
    }

    #[test]
    fn where_operators() {
        let (sql, binds) = compile(&seg(json!({"where": [
            {"field": "data.seats", "op": "gte", "value": 5},
            {"field": "data.country", "op": "not_in", "value": ["US"]},
            {"field": "data.company.size", "op": "exists", "value": true},
            {"field": "email", "op": "contains", "value": "@acme.com"},
            {"field": "created_at", "op": "lt", "value": "2026-06-01T00:00:00Z"},
            {"field": "data.plan", "op": "neq", "value": "free"}
        ]})))
        .unwrap();
        assert!(sql.contains("CASE WHEN data->>'seats' ~ '^-?[0-9]+(\\.[0-9]+)?$' THEN (data->>'seats')::numeric END >= ($2)::numeric"));
        assert!(
            sql.contains("(data->>'country' IS NULL OR NOT (data->>'country' = ANY($3::text[])))")
        );
        assert!(sql.contains("data#>>'{company,size}' IS NOT NULL"));
        assert!(sql.contains("email ILIKE $4"));
        assert!(sql.contains("created_at < $5"));
        assert!(sql.contains("(data->>'plan' IS NULL OR data->>'plan' <> $6)"));
        assert_eq!(binds[2], Bind::Text("%@acme.com%".into()));
    }

    #[test]
    fn rejects_unknown_fields_operators_and_bad_keys() {
        assert!(compile(&seg(
            json!({"where": [{"field": "password", "op": "eq", "value": "x"}]})
        ))
        .is_err());
        assert!(compile(&seg(
            json!({"where": [{"field": "email", "op": "like", "value": "x"}]})
        ))
        .is_err());
        assert!(compile(&seg(
            json!({"data": {"plan'; DROP TABLE subscribers; --": "pro"}})
        ))
        .is_err());
        assert!(compile(&seg(
            json!({"where": [{"field": "data.seats", "op": "gt", "value": "five"}]})
        ))
        .is_err());
        assert!(compile(&seg(json!({"locale": []}))).is_err());
        assert!(compile(&seg(
            json!({"where": [{"field": "data.x", "op": "in", "value": "a"}]})
        ))
        .is_err());
    }

    #[test]
    fn empty_segment_means_everyone_in_the_project() {
        let (sql, binds) = compile(&Segment::default()).unwrap();
        assert_eq!(sql, "project_id = $1");
        assert!(binds.is_empty());
    }
}
