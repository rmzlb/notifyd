//! Outbound pacing: one token bucket per channel, plus a pause set when a
//! provider answers 429. The worker asks the pacer before every provider
//! request (a batch call is one request) and skips the channels that are
//! paused when it claims jobs, so a throttled lane stops hammering the
//! provider while the other lanes keep flowing.
//!
//! State is per process. With several replicas the effective rate is
//! N × `rate_per_sec`: size the per-replica rate accordingly.

use crate::config::PacingConfig;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Lane {
    rate_per_sec: f64,
    tokens: f64,
    burst: f64,
    refilled_at: Instant,
    paused_until: Option<Instant>,
}

impl Lane {
    fn new(rate_per_sec: f64, now: Instant) -> Self {
        let burst = rate_per_sec.max(1.0);
        Self {
            rate_per_sec,
            tokens: burst,
            burst,
            refilled_at: now,
            paused_until: None,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.refilled_at)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.burst);
        self.refilled_at = now;
    }

    /// Time to wait before one request may go out. Zero when a token is
    /// available now (and it is consumed).
    fn acquire(&mut self, now: Instant) -> Duration {
        if let Some(until) = self.paused_until {
            if until > now {
                return until - now;
            }
            self.paused_until = None;
        }
        if self.rate_per_sec <= 0.0 {
            return Duration::ZERO;
        }
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Duration::ZERO
        } else {
            let missing = 1.0 - self.tokens;
            Duration::from_secs_f64(missing / self.rate_per_sec)
        }
    }
}

#[derive(Debug)]
pub struct Pacer {
    lanes: Mutex<HashMap<String, Lane>>,
    default_pause: Duration,
    config: PacingConfig,
}

impl Pacer {
    pub fn new(config: PacingConfig) -> Self {
        Self {
            lanes: Mutex::new(HashMap::new()),
            default_pause: Duration::from_secs(config.rate_limit_pause_secs.max(1)),
            config,
        }
    }

    fn rate_for(&self, channel: &str) -> f64 {
        match channel {
            "email" => self.config.email_per_sec,
            "sms" => self.config.sms_per_sec,
            "whatsapp" => self.config.whatsapp_per_sec,
            "push" => self.config.push_per_sec,
            _ => 0.0, // in_app writes to our own database: never paced
        }
    }

    fn with_lane<T>(&self, channel: &str, now: Instant, f: impl FnOnce(&mut Lane) -> T) -> T {
        let mut lanes = self.lanes.lock().expect("pacer mutex");
        let lane = lanes
            .entry(channel.to_string())
            .or_insert_with(|| Lane::new(self.rate_for(channel), now));
        f(lane)
    }

    /// Wait until the lane lets one provider request through.
    pub async fn acquire(&self, channel: &str) {
        loop {
            let wait = self.with_lane(channel, Instant::now(), |lane| lane.acquire(Instant::now()));
            if wait.is_zero() {
                return;
            }
            tokio::time::sleep(wait.min(Duration::from_secs(30))).await;
        }
    }

    /// Pause a lane after a 429. `retry_after` comes from the provider when
    /// it sent one; otherwise the configured default pause applies.
    pub fn pause(&self, channel: &str, retry_after: Option<Duration>) -> Duration {
        let pause = retry_after
            .unwrap_or(self.default_pause)
            .max(Duration::from_millis(500));
        let now = Instant::now();
        self.with_lane(channel, now, |lane| {
            let until = now + pause;
            if lane.paused_until.map(|u| until > u).unwrap_or(true) {
                lane.paused_until = Some(until);
            }
        });
        pause
    }

    /// Channels that must not be claimed right now.
    pub fn paused_channels(&self) -> Vec<String> {
        let now = Instant::now();
        let lanes = self.lanes.lock().expect("pacer mutex");
        lanes
            .iter()
            .filter(|(_, lane)| lane.paused_until.map(|u| u > now).unwrap_or(false))
            .map(|(channel, _)| channel.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pacer(email_per_sec: f64) -> Pacer {
        Pacer::new(PacingConfig {
            email_per_sec,
            sms_per_sec: 10.0,
            whatsapp_per_sec: 10.0,
            push_per_sec: 50.0,
            rate_limit_pause_secs: 2,
        })
    }

    #[test]
    fn bucket_allows_burst_then_throttles() {
        let now = Instant::now();
        let mut lane = Lane::new(2.0, now);
        assert_eq!(lane.acquire(now), Duration::ZERO);
        assert_eq!(lane.acquire(now), Duration::ZERO);
        let wait = lane.acquire(now);
        assert!(wait > Duration::from_millis(400) && wait <= Duration::from_millis(500));
        // Half a second later one token has been refilled.
        assert_eq!(
            lane.acquire(now + Duration::from_millis(500)),
            Duration::ZERO
        );
    }

    #[test]
    fn zero_rate_means_unpaced() {
        let now = Instant::now();
        let mut lane = Lane::new(0.0, now);
        for _ in 0..1000 {
            assert_eq!(lane.acquire(now), Duration::ZERO);
        }
    }

    #[test]
    fn pause_blocks_lane_and_is_reported() {
        let pacer = pacer(8.0);
        assert!(pacer.paused_channels().is_empty());
        let pause = pacer.pause("email", Some(Duration::from_secs(3)));
        assert_eq!(pause, Duration::from_secs(3));
        assert_eq!(pacer.paused_channels(), vec!["email".to_string()]);
        // A shorter pause never shortens an existing one.
        pacer.pause("email", Some(Duration::from_millis(600)));
        let now = Instant::now();
        let wait = pacer.with_lane("email", now, |lane| lane.acquire(now));
        assert!(wait > Duration::from_secs(2));
    }

    #[test]
    fn default_pause_applies_without_retry_after() {
        let pacer = pacer(8.0);
        assert_eq!(pacer.pause("sms", None), Duration::from_secs(2));
    }

    #[test]
    fn in_app_is_never_paced() {
        let pacer = pacer(8.0);
        let now = Instant::now();
        for _ in 0..100 {
            assert_eq!(
                pacer.with_lane("in_app", now, |l| l.acquire(now)),
                Duration::ZERO
            );
        }
    }
}
