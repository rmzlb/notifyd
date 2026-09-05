//! Send windows: bulk email waits for the recipient's daytime instead of
//! landing at 3 a.m. A window is a daily [start, end) range of local time in
//! an IANA timezone, optionally restricted to weekdays. The recipient's own
//! timezone (`subscribers.timezone`) wins over the window's default.
//!
//! Configured per project (`settings.send_window`) or per request
//! (`send_window` on `/v1/send` and `/v1/batch`; `false` bypasses). By default
//! it applies to marketing email only; `"applies_to": "all"` widens it.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct SendWindowSpec {
    /// "09:00"
    pub start: String,
    /// "20:00", must be after `start`
    pub end: String,
    /// IANA zone used when the recipient has none, e.g. "Europe/Paris"
    #[serde(default = "default_tz")]
    pub tz: String,
    /// ISO weekdays 1 (Monday) … 7 (Sunday); default every day
    #[serde(default)]
    pub days: Option<Vec<u8>>,
    /// "marketing" (default) or "all"
    #[serde(default)]
    pub applies_to: Option<String>,
}

fn default_tz() -> String {
    "UTC".to_string()
}

#[derive(Debug, Clone)]
pub struct SendWindow {
    start: NaiveTime,
    end: NaiveTime,
    tz: Tz,
    days: Vec<Weekday>,
    pub applies_to_all: bool,
}

impl SendWindow {
    pub fn parse(value: &Value) -> Result<Self> {
        let spec: SendWindowSpec =
            serde_json::from_value(value.clone()).map_err(|e| anyhow!("send_window: {e}"))?;
        let start = NaiveTime::parse_from_str(&spec.start, "%H:%M")
            .map_err(|_| anyhow!("send_window.start must be HH:MM"))?;
        let end = NaiveTime::parse_from_str(&spec.end, "%H:%M")
            .map_err(|_| anyhow!("send_window.end must be HH:MM"))?;
        if end <= start {
            return Err(anyhow!("send_window.end must be after start (same day)"));
        }
        let tz: Tz = spec
            .tz
            .parse()
            .map_err(|_| anyhow!("send_window.tz must be an IANA timezone (e.g. Europe/Paris)"))?;
        let days = match spec.days {
            None => vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ],
            Some(list) => {
                if list.is_empty() {
                    return Err(anyhow!("send_window.days must not be empty"));
                }
                list.iter()
                    .map(|d| match d {
                        1 => Ok(Weekday::Mon),
                        2 => Ok(Weekday::Tue),
                        3 => Ok(Weekday::Wed),
                        4 => Ok(Weekday::Thu),
                        5 => Ok(Weekday::Fri),
                        6 => Ok(Weekday::Sat),
                        7 => Ok(Weekday::Sun),
                        other => Err(anyhow!("send_window.days: {other} is not 1..7")),
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        Ok(Self {
            start,
            end,
            tz,
            days,
            applies_to_all: spec.applies_to.as_deref() == Some("all"),
        })
    }

    /// The earliest instant ≥ `not_before` inside the window, in the
    /// recipient's zone when known (IANA name), else the window's zone.
    pub fn next_allowed(
        &self,
        not_before: DateTime<Utc>,
        recipient_tz: Option<&str>,
    ) -> DateTime<Utc> {
        let tz = recipient_tz
            .and_then(|name| name.parse::<Tz>().ok())
            .unwrap_or(self.tz);
        let local = not_before.with_timezone(&tz);
        for day_offset in 0..8 {
            let date = local.date_naive() + Duration::days(day_offset);
            if !self.days.contains(&date.weekday()) {
                continue;
            }
            let open = match tz
                .from_local_datetime(&date.and_time(self.start))
                .earliest()
            {
                Some(t) => t,
                None => continue,
            };
            let close = match tz.from_local_datetime(&date.and_time(self.end)).earliest() {
                Some(t) => t,
                None => continue,
            };
            if day_offset == 0 && local >= open && local < close {
                return not_before;
            }
            if open > local {
                return open.with_timezone(&Utc);
            }
        }
        not_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn window(days: Option<Vec<u8>>) -> SendWindow {
        SendWindow::parse(
            &json!({"start": "09:00", "end": "20:00", "tz": "Europe/Paris", "days": days}),
        )
        .unwrap()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn inside_window_sends_now() {
        let w = window(None);
        let now = utc("2026-09-08T12:00:00Z"); // 14:00 Paris, Tuesday
        assert_eq!(w.next_allowed(now, None), now);
    }

    #[test]
    fn night_waits_for_morning_in_paris() {
        let w = window(None);
        let now = utc("2026-09-08T01:00:00Z"); // 03:00 Paris
        assert_eq!(w.next_allowed(now, None), utc("2026-09-08T07:00:00Z")); // 09:00 CEST
    }

    #[test]
    fn after_close_waits_for_next_day() {
        let w = window(None);
        let now = utc("2026-09-08T19:30:00Z"); // 21:30 Paris
        assert_eq!(w.next_allowed(now, None), utc("2026-09-09T07:00:00Z"));
    }

    #[test]
    fn weekdays_only_skip_weekend() {
        let w = window(Some(vec![1, 2, 3, 4, 5]));
        let now = utc("2026-09-12T12:00:00Z"); // Saturday
        assert_eq!(w.next_allowed(now, None), utc("2026-09-14T07:00:00Z")); // Monday 09:00
    }

    #[test]
    fn recipient_timezone_wins() {
        let w = window(None);
        let now = utc("2026-09-08T01:00:00Z"); // 21:00 in New York the day before
        assert_eq!(
            w.next_allowed(now, Some("America/New_York")),
            utc("2026-09-08T13:00:00Z") // 09:00 EDT
        );
        // Unknown zone falls back to the window's zone.
        assert_eq!(
            w.next_allowed(now, Some("Mars/Olympus")),
            utc("2026-09-08T07:00:00Z")
        );
    }

    #[test]
    fn validation() {
        assert!(SendWindow::parse(&json!({"start": "20:00", "end": "09:00"})).is_err());
        assert!(SendWindow::parse(&json!({"start": "9h", "end": "20:00"})).is_err());
        assert!(
            SendWindow::parse(&json!({"start": "09:00", "end": "20:00", "tz": "Nowhere"})).is_err()
        );
        assert!(
            SendWindow::parse(&json!({"start": "09:00", "end": "20:00", "days": [8]})).is_err()
        );
        assert!(
            SendWindow::parse(&json!({"start": "09:00", "end": "20:00", "applies_to": "all"}))
                .unwrap()
                .applies_to_all
        );
    }
}
