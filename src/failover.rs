//! Provider failover for email: a circuit breaker in front of the primary
//! connector. A 429 or a transient error trips it; while it is open the
//! worker sends through the fallback connector, then the primary is tried
//! again after the cooldown. Permanent errors never trip it: a bad address
//! fails the same way at every provider.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct Breaker {
    open_until: Mutex<Option<Instant>>,
    trips: Mutex<u64>,
}

impl Breaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the primary should not be used.
    pub fn is_open(&self) -> bool {
        let mut guard = self.open_until.lock().expect("breaker mutex");
        match *guard {
            Some(until) if until > Instant::now() => true,
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    /// Open the breaker for `cooldown`. Extends an open breaker, never
    /// shortens it.
    pub fn trip(&self, cooldown: Duration) {
        let until = Instant::now() + cooldown;
        let mut guard = self.open_until.lock().expect("breaker mutex");
        if guard.map(|u| until > u).unwrap_or(true) {
            *guard = Some(until);
        }
        *self.trips.lock().expect("breaker mutex") += 1;
    }

    pub fn trips(&self) -> u64 {
        *self.trips.lock().expect("breaker mutex")
    }

    /// Seconds left before the primary is tried again, if open.
    pub fn open_for(&self) -> Option<Duration> {
        let guard = self.open_until.lock().expect("breaker mutex");
        guard.and_then(|u| u.checked_duration_since(Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_opens_and_closes() {
        let b = Breaker::new();
        assert!(!b.is_open());
        b.trip(Duration::from_millis(50));
        assert!(b.is_open());
        assert_eq!(b.trips(), 1);
        std::thread::sleep(Duration::from_millis(70));
        assert!(!b.is_open());
        assert!(b.open_for().is_none());
    }

    #[test]
    fn trip_never_shortens() {
        let b = Breaker::new();
        b.trip(Duration::from_secs(60));
        b.trip(Duration::from_millis(10));
        assert!(b.open_for().unwrap() > Duration::from_secs(50));
    }
}
