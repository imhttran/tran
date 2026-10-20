//! In-process rate limiting for sensitive operations.
//!
//! Rate limits are evaluated after authentication and authorization, keyed by
//! the verified subject and the operation bucket. Unauthenticated traffic never
//! consumes a privileged quota, and client-supplied identity headers or body
//! fields cannot select a different bucket.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A fixed-window limiter per `(subject, operation)`.
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<(String, &'static str), Bucket>>,
}

struct Bucket {
    window_start: Instant,
    count: u32,
}

impl RateLimiter {
    /// Builds a limiter allowing `limit` requests per `window` for each key.
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Builds a limiter that never rejects (used when a limit is zero).
    pub fn disabled() -> Self {
        Self::new(u32::MAX, Duration::from_secs(1))
    }

    /// Records one request and reports whether it is allowed.
    pub fn check(&self, subject: &str, operation: &'static str) -> bool {
        let mut buckets = match self.buckets.lock() {
            Ok(buckets) => buckets,
            Err(_) => return false,
        };
        let now = Instant::now();
        let key = (subject.to_string(), operation);
        let bucket = buckets.entry(key).or_insert(Bucket {
            window_start: now,
            count: 0,
        });
        if now.duration_since(bucket.window_start) >= self.window {
            bucket.window_start = now;
            bucket.count = 0;
        }
        if bucket.count >= self.limit {
            return false;
        }
        bucket.count += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exceeding_the_limit_is_rejected() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("user-1", "query"));
        assert!(limiter.check("user-1", "query"));
        assert!(!limiter.check("user-1", "query"));
    }

    #[test]
    fn buckets_are_per_subject_and_operation() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("user-1", "query"));
        assert!(!limiter.check("user-1", "query"));
        assert!(limiter.check("user-2", "query"));
        assert!(limiter.check("user-1", "ingest"));
    }

    #[test]
    fn window_resets() {
        let limiter = RateLimiter::new(1, Duration::from_millis(1));
        assert!(limiter.check("user-1", "query"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(limiter.check("user-1", "query"));
    }
}
