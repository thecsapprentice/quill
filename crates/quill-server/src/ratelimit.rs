//! Small in-memory rate limiter.
//!
//! Keys are account ids or fixed buckets such as "all anonymous reports".
//! Client IP addresses are deliberately never used or stored, so anonymous
//! actions share one server-wide budget instead of a per-address one.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    /// Records an event for `key` and returns false if more than `max` events
    /// already happened within `window`.
    pub fn allow(&self, key: &str, max: usize, window: Duration) -> bool {
        self.allow_at(key, max, window, Instant::now())
    }

    fn allow_at(&self, key: &str, max: usize, window: Duration, now: Instant) -> bool {
        let mut windows = match self.windows.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Bound memory: drop keys whose events have all expired.
        if windows.len() > 10_000 {
            windows.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < window));
        }
        let events = windows.entry(key.to_owned()).or_default();
        while events
            .front()
            .is_some_and(|t| now.duration_since(*t) >= window)
        {
            events.pop_front();
        }
        if events.len() >= max {
            return false;
        }
        events.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_within_window_and_recovers() {
        let rl = RateLimiter::default();
        let w = Duration::from_secs(60);
        let t0 = Instant::now();
        assert!(rl.allow_at("k", 2, w, t0));
        assert!(rl.allow_at("k", 2, w, t0));
        assert!(!rl.allow_at("k", 2, w, t0 + Duration::from_secs(1)));
        assert!(rl.allow_at("other", 2, w, t0));
        assert!(rl.allow_at("k", 2, w, t0 + Duration::from_secs(61)));
    }
}
